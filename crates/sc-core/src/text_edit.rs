//! The editor's own commands on the text: its language by the file's name,
//! its indentation, and changes to lines and case, as VS Code makes them.

use std::ops::Range;
use std::path::Path;

/// The Language menu: GPUI Kit's highlighters, by their names, and XML's,
/// which the app adds.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("Plain Text", "text"),
    ("Astro", "astro"),
    ("Bash", "bash"),
    ("C", "c"),
    ("C#", "csharp"),
    ("C++", "cpp"),
    ("CMake", "cmake"),
    ("CSS", "css"),
    ("Diff", "diff"),
    ("EJS", "ejs"),
    ("Elixir", "elixir"),
    ("ERB", "erb"),
    ("Go", "go"),
    ("GraphQL", "graphql"),
    ("HTML", "html"),
    ("Java", "java"),
    ("JavaScript", "javascript"),
    ("JSON", "json"),
    ("Kotlin", "kotlin"),
    ("Lua", "lua"),
    ("Make", "make"),
    ("Markdown", "markdown"),
    ("PHP", "php"),
    ("Protocol Buffers", "proto"),
    ("Python", "python"),
    ("Ruby", "ruby"),
    ("Rust", "rust"),
    ("Scala", "scala"),
    ("SQL", "sql"),
    ("Svelte", "svelte"),
    ("Swift", "swift"),
    ("TOML", "toml"),
    ("TSX", "tsx"),
    ("TypeScript", "typescript"),
    ("XML", "xml"),
    ("YAML", "yaml"),
    ("Zig", "zig"),
];

/// The language a file is highlighted as, by its name: "text" for none.
pub fn language_of(path: &Path) -> &'static str {
    let lower = |s: Option<&std::ffi::OsStr>| s.map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    match lower(path.file_name()).as_str() {
        "makefile" | "gnumakefile" => return "make",
        "cmakelists.txt" => return "cmake",
        ".bashrc" | ".bash_profile" | ".bash_aliases" | ".profile" | ".zshrc" | "pkgbuild" => return "bash",
        _ => {}
    }
    match lower(path.extension()).as_str() {
        "astro" => "astro",
        "sh" | "bash" | "zsh" | "ksh" => "bash",
        "c" | "h" => "c",
        "cs" => "csharp",
        "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "ino" => "cpp",
        "cmake" => "cmake",
        "css" | "scss" => "css",
        "diff" | "patch" => "diff",
        "ejs" => "ejs",
        "ex" | "exs" => "elixir",
        "erb" => "erb",
        "go" => "go",
        "graphql" | "gql" => "graphql",
        "html" | "htm" | "xhtml" => "html",
        "java" => "java",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "json" | "jsonc" | "geojson" | "ipynb" => "json",
        "kt" | "kts" => "kotlin",
        "lua" => "lua",
        "mk" | "mak" => "make",
        "md" | "markdown" | "mdx" => "markdown",
        "php" | "phtml" => "php",
        "proto" => "proto",
        "py" | "pyi" | "pyw" => "python",
        "rb" | "rake" | "gemspec" => "ruby",
        "rs" => "rust",
        "scala" | "sc" => "scala",
        "sql" => "sql",
        "svelte" => "svelte",
        "swift" => "swift",
        "toml" => "toml",
        "tsx" => "tsx",
        "ts" | "mts" | "cts" => "typescript",
        "xml" | "xsd" | "xsl" | "xslt" | "svg" | "plist" | "rss" | "atom" | "wsdl" | "kml" | "gpx" | "xaml" | "resx"
        | "csproj" | "vbproj" | "fsproj" | "vcxproj" | "props" | "targets" | "nuspec" | "xlf" => "xml",
        "yaml" | "yml" => "yaml",
        "zig" => "zig",
        _ => "text",
    }
}

/// What starts a line comment in `language`; none where it has only block comments.
pub fn line_comment(language: &str) -> Option<&'static str> {
    match language {
        "c" | "cpp" | "csharp" | "go" | "java" | "javascript" | "typescript" | "tsx" | "kotlin" | "rust" | "scala"
        | "swift" | "zig" | "php" | "proto" => Some("//"),
        "bash" | "python" | "ruby" | "toml" | "yaml" | "make" | "cmake" | "elixir" | "graphql" => Some("#"),
        "sql" | "lua" => Some("--"),
        _ => None,
    }
}

/// How the text indents, as VS Code guesses it: tabs, or the spaces most
/// steps in (2, 4 or 8); none when no line is indented.
pub fn detect_indent(text: &str) -> Option<(usize, bool)> {
    let (mut tabs, mut spaces, mut steps) = (0, 0, [0usize; 3]);
    let mut previous = 0;
    for line in text.lines().take(10_000).filter(|line| !line.trim().is_empty()) {
        let indent = line.len() - line.trim_start_matches(' ').len();
        if line.starts_with('\t') {
            tabs += 1;
        } else if indent > 0 {
            spaces += 1;
        }
        if indent > previous {
            match indent - previous {
                2 => steps[0] += 1,
                4 => steps[1] += 1,
                8 => steps[2] += 1,
                _ => {}
            }
        }
        previous = indent;
    }
    match () {
        _ if tabs > spaces => Some((4, true)),
        _ if spaces == 0 => None,
        // Ties go to the wider step: 4 over 2.
        _ => {
            let best = (0..3).max_by_key(|&ix| (steps[ix], ix == 1)).unwrap_or(1);
            Some(([2, 4, 8][best], false))
        }
    }
}

/// A change to the text: what replaces `range`, and the selection after it.
#[derive(Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub range: Range<usize>,
    pub text: String,
    pub selection: Range<usize>,
}

impl TextEdit {
    /// The text with the change made.
    pub fn apply(&self, text: &str) -> String {
        format!("{}{}{}", &text[..self.range.start], self.text, &text[self.range.end..])
    }
}

/// The text's line end: CRLF where it has one.
fn eol(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// The whole lines `selection` touches, their last line end left out; a
/// selection that ends where a line starts leaves that line out.
fn lines_around(text: &str, selection: &Range<usize>) -> Range<usize> {
    let mut end = selection.end;
    if end > selection.start && text[..end].ends_with('\n') {
        end -= 1;
    }
    let start = text[..selection.start].rfind('\n').map_or(0, |ix| ix + 1);
    let mut end = text[end..].find('\n').map_or(text.len(), |ix| end + ix);
    if end > start && text[..end].ends_with('\r') && text[end..].starts_with('\n') {
        end -= 1;
    }
    start..end.max(start)
}

/// The lines of `block`, their CRs left out.
fn lines(block: &str) -> Vec<&str> {
    block.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)).collect()
}

fn shifted(selection: &Range<usize>, by: isize) -> Range<usize> {
    let shift = |at: usize| at.saturating_add_signed(by);
    shift(selection.start)..shift(selection.end)
}

/// The lines copied below them, the copy selected.
pub fn duplicate_lines(text: &str, selection: Range<usize>) -> TextEdit {
    let block = lines_around(text, &selection);
    let eol = eol(text);
    let copy = format!("{eol}{}", &text[block.clone()]);
    let by = copy.len() as isize;
    TextEdit { range: block.end..block.end, text: copy, selection: shifted(&selection, by) }
}

/// The lines gone, the caret where the next one starts.
pub fn delete_lines(text: &str, selection: Range<usize>) -> TextEdit {
    let block = lines_around(text, &selection);
    let after = &text[block.end..];
    let range = match after.strip_prefix("\r\n").or_else(|| after.strip_prefix('\n')) {
        Some(rest) => block.start..text.len() - rest.len(),
        // The last line: the line end before it goes too.
        None => {
            let before = &text[..block.start];
            let start = before.strip_suffix("\r\n").or_else(|| before.strip_suffix('\n')).map_or(block.start, str::len);
            start..block.end
        }
    };
    let caret = match range.start < block.start {
        true => text[..range.start].rfind('\n').map_or(0, |ix| ix + 1),
        false => range.start,
    };
    TextEdit { range, text: String::new(), selection: caret..caret }
}

/// The lines moved above the line before them, or below the one after; none
/// at the text's start or end.
pub fn move_lines(text: &str, selection: Range<usize>, up: bool) -> Option<TextEdit> {
    let block = lines_around(text, &selection);
    let eol = eol(text);
    let moved = &text[block.clone()];
    if up {
        let before = &text[..block.start];
        let end = before.strip_suffix("\r\n").or_else(|| before.strip_suffix('\n'))?.len();
        let start = text[..end].rfind('\n').map_or(0, |ix| ix + 1);
        let other = &text[start..end];
        let by = -((other.len() + eol.len()) as isize);
        Some(TextEdit { range: start..block.end, text: format!("{moved}{eol}{other}"), selection: shifted(&selection, by) })
    } else {
        let after = &text[block.end..];
        let rest = after.strip_prefix("\r\n").or_else(|| after.strip_prefix('\n'))?;
        let start = text.len() - rest.len();
        let end = lines_around(text, &(start..start)).end;
        let other = &text[start..end];
        let by = (other.len() + eol.len()) as isize;
        Some(TextEdit { range: block.start..end, text: format!("{other}{eol}{moved}"), selection: shifted(&selection, by) })
    }
}

/// The selected lines sorted, or every line where at most one is selected.
pub fn sort_lines(text: &str, selection: Range<usize>, descending: bool) -> TextEdit {
    let mut block = lines_around(text, &selection);
    if !text[block.clone()].contains('\n') {
        block = lines_around(text, &(0..text.len()));
    }
    let mut sorted = lines(&text[block.clone()]);
    sorted.sort();
    if descending {
        sorted.reverse();
    }
    let joined = sorted.join(eol(text));
    let selection = block.start..block.start + joined.len();
    TextEdit { range: block, text: joined, selection }
}

/// Every line without its trailing spaces and tabs; none when none has any.
pub fn trim_trailing_whitespace(text: &str, selection: Range<usize>) -> Option<TextEdit> {
    let trimmed: String = text
        .split_inclusive('\n')
        .map(|line| {
            let body = line.trim_end_matches(['\n', '\r']);
            format!("{}{}", body.trim_end_matches([' ', '\t']), &line[body.len()..])
        })
        .collect();
    if trimmed == text {
        return None;
    }
    let map = |offset: usize| map_through_trim(text, offset);
    Some(TextEdit { range: 0..text.len(), text: trimmed, selection: map(selection.start)..map(selection.end) })
}

/// Where `offset` is once trailing whitespace is trimmed from every line.
fn map_through_trim(text: &str, offset: usize) -> usize {
    let mut removed = 0;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let trimmed = at + body.trim_end_matches([' ', '\t']).len();
        let body_end = at + body.len();
        if offset <= trimmed {
            return offset - removed;
        }
        if offset <= body_end {
            return trimmed - removed;
        }
        removed += body_end - trimmed;
        at += line.len();
    }
    offset - removed
}

/// The lines commented out with `prefix` at their least indent, or, where
/// every one not blank is already, uncommented.
pub fn toggle_comment(text: &str, selection: Range<usize>, prefix: &str) -> TextEdit {
    let block = lines_around(text, &selection);
    let original = &text[block.clone()];
    let rows: Vec<&str> = original.split('\n').collect();
    let filled = |line: &str| !line.trim().is_empty();
    let commented = rows.iter().filter(|line| filled(line)).all(|line| line.trim_start().starts_with(prefix));
    let indent = rows.iter().filter(|line| filled(line)).map(|line| line.len() - line.trim_start().len()).min().unwrap_or(0);
    let changed: Vec<String> = rows
        .iter()
        .map(|line| match (filled(line), commented) {
            (false, _) => line.to_string(),
            (true, true) => {
                let lead = line.len() - line.trim_start().len();
                let rest = &line[lead + prefix.len()..];
                format!("{}{}", &line[..lead], rest.strip_prefix(' ').unwrap_or(rest))
            }
            (true, false) => format!("{}{prefix} {}", &line[..indent], &line[indent..]),
        })
        .collect();
    let joined = changed.join("\n");
    let selection = match selection.is_empty() && !original.contains('\n') {
        // A caret keeps its place in the line's text.
        true => {
            let by = joined.len() as isize - original.len() as isize;
            let caret = selection.start.saturating_add_signed(by).max(block.start);
            caret..caret
        }
        false => block.start..block.start + joined.len(),
    };
    TextEdit { range: block, text: joined, selection }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextCase {
    Upper,
    Lower,
    Title,
}

/// The selection, or the word at the caret, in `case`; none where there is
/// neither.
pub fn change_case(text: &str, selection: Range<usize>, case: TextCase) -> Option<TextEdit> {
    let range = match selection.is_empty() {
        true => crate::text_find::word_at(text, selection.start),
        false => selection.clone(),
    };
    if range.is_empty() {
        return None;
    }
    let part = &text[range.clone()];
    let changed = match case {
        TextCase::Upper => part.to_uppercase(),
        TextCase::Lower => part.to_lowercase(),
        TextCase::Title => {
            let mut start = true;
            part.chars()
                .flat_map(|c| {
                    let out: Vec<char> = match start {
                        true => c.to_uppercase().collect(),
                        false => c.to_lowercase().collect(),
                    };
                    start = !c.is_alphanumeric() && c != '\'';
                    out
                })
                .collect()
        }
    };
    let selection = match selection.is_empty() {
        true => selection,
        false => range.start..range.start + changed.len(),
    };
    Some(TextEdit { range, text: changed, selection })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, edit: TextEdit) -> (String, String) {
        let out = edit.apply(text);
        let selected = out[edit.selection.clone()].to_owned();
        (out, selected)
    }

    #[test]
    fn languages_by_name() {
        assert_eq!(language_of(Path::new("src/main.RS")), "rust");
        assert_eq!(language_of(Path::new("Makefile")), "make");
        assert_eq!(language_of(Path::new("CMakeLists.txt")), "cmake");
        assert_eq!(language_of(Path::new("notes.txt")), "text");
        assert_eq!(language_of(Path::new("README")), "text");
        assert_eq!(language_of(Path::new("pom.xml")), "xml");
        assert_eq!(line_comment("rust"), Some("//"));
        assert_eq!(line_comment("markdown"), None);
        assert!(LANGUAGES.iter().any(|(_, name)| *name == language_of(Path::new("a.yml"))));
    }

    #[test]
    fn indentation_guessed() {
        assert_eq!(detect_indent("a\n\tb\n\tc\n"), Some((4, true)));
        assert_eq!(detect_indent("a\n  b\n    c\n  d\n"), Some((2, false)));
        assert_eq!(detect_indent("a\n    b\n        c\n"), Some((4, false)));
        assert_eq!(detect_indent("a\nb\n"), None);
    }

    #[test]
    fn lines_duplicated_deleted_and_moved() {
        let text = "one\ntwo\nthree";
        assert_eq!(run(text, duplicate_lines(text, 5..5)).0, "one\ntwo\ntwo\nthree");
        assert_eq!(run(text, duplicate_lines(text, 0..6)), ("one\ntwo\none\ntwo\nthree".into(), "one\ntw".into()));
        assert_eq!(delete_lines(text, 5..5).apply(text), "one\nthree");
        assert_eq!(delete_lines(text, 9..9).apply(text), "one\ntwo");
        assert_eq!(delete_lines(text, 9..9).selection, 4..4);
        // A selection ending where a line starts leaves that line be.
        assert_eq!(delete_lines(text, 0..4).apply(text), "two\nthree");
        assert_eq!(run(text, move_lines(text, 4..7, true).unwrap()), ("two\none\nthree".into(), "two".into()));
        assert_eq!(run(text, move_lines(text, 4..7, false).unwrap()), ("one\nthree\ntwo".into(), "two".into()));
        assert!(move_lines(text, 0..0, true).is_none());
        assert!(move_lines(text, 9..9, false).is_none());
        let crlf = "a\r\nb\r\nc";
        assert_eq!(move_lines(crlf, 6..6, true).unwrap().apply(crlf), "a\r\nc\r\nb");
        assert_eq!(duplicate_lines(crlf, 0..0).apply(crlf), "a\r\na\r\nb\r\nc");
    }

    #[test]
    fn lines_sorted() {
        let text = "pear\napple\nfig\n";
        assert_eq!(sort_lines(text, 0..0, false).apply(text), "apple\nfig\npear\n");
        assert_eq!(sort_lines(text, 0..10, true).apply(text), "pear\napple\nfig\n");
        assert_eq!(sort_lines(text, 0..10, false).apply(text), "apple\npear\nfig\n");
    }

    #[test]
    fn trailing_whitespace_trimmed() {
        let text = "a  \nb\t\r\nc";
        let edit = trim_trailing_whitespace(text, 8..8).unwrap();
        assert_eq!(edit.apply(text), "a\nb\r\nc");
        assert_eq!(edit.selection, 5..5);
        assert!(trim_trailing_whitespace("a\nb", 0..0).is_none());
    }

    #[test]
    fn comments_toggled() {
        let text = "fn a() {\n    one();\n\n    two();\n}";
        let edit = toggle_comment(text, 9..30, "//");
        let commented = edit.apply(text);
        assert_eq!(commented, "fn a() {\n    // one();\n\n    // two();\n}");
        let back = toggle_comment(&commented, edit.selection, "//");
        assert_eq!(back.apply(&commented), text);
        let caret = toggle_comment("x = 1", 2..2, "#");
        assert_eq!((caret.apply("x = 1"), caret.selection), ("# x = 1".into(), 4..4));
    }

    #[test]
    fn case_changed() {
        let text = "hello wORLD";
        assert_eq!(run(text, change_case(text, 0..11, TextCase::Upper).unwrap()).1, "HELLO WORLD");
        assert_eq!(run(text, change_case(text, 0..11, TextCase::Title).unwrap()).1, "Hello World");
        // The word at the caret, the caret left where it was.
        let edit = change_case(text, 8..8, TextCase::Lower).unwrap();
        assert_eq!((edit.apply(text), edit.selection), ("hello world".into(), 8..8));
        assert!(change_case("a  b", 2..2, TextCase::Upper).is_none());
        assert_eq!(change_case("straße", 0..7, TextCase::Upper).unwrap().text, "STRASSE");
    }
}
