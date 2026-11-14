//! The editor's Find and Replace, as VS Code's find does (taken over from
//! Water Data Studio): plain, whole words or a regular expression, with or
//! without case, and what replaces a match.

use std::ops::Range;

use regex::{Regex, RegexBuilder};

/// A text as a regular expression that matches it, as a search's seed.
pub use regex::escape;

/// More are not counted.
pub const LIMIT: usize = 20_000;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Options {
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub preserve_case: bool,
}

pub struct Finder {
    regex: Regex,
    options: Options,
}

impl Finder {
    /// `None` for no query; an error for a pattern that does not compile.
    pub fn new(query: &str, options: Options) -> Option<Result<Self, String>> {
        if query.is_empty() {
            return None;
        }
        let mut pattern = match options.regex {
            true => query.to_owned(),
            false => regex::escape(query),
        };
        if options.whole_word {
            pattern = format!(r"\b(?:{pattern})\b");
        }
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(!options.match_case)
            .multi_line(true)
            .build()
            .map_err(|err| match err {
                // The last line, "error: unclosed group".
                regex::Error::Syntax(text) => {
                    let last = text.lines().last().unwrap_or_default();
                    let mut chars = last.trim_start_matches("error: ").chars();
                    chars
                        .next()
                        .map(|first| first.to_uppercase().chain(chars).collect())
                        .unwrap_or_default()
                }
                err => err.to_string(),
            });
        Some(regex.map(|regex| Self { regex, options }))
    }

    /// The matches in `text[scope]`, empty ones left out, at most `LIMIT`.
    pub fn matches(&self, text: &str, scope: Range<usize>) -> Vec<Range<usize>> {
        self.regex
            .find_iter(&text[scope.clone()])
            .filter(|m| !m.is_empty())
            .take(LIMIT)
            .map(|m| scope.start + m.start()..scope.start + m.end())
            .collect()
    }

    /// What replaces the match at `found`: `$1` and `${name}` are its groups
    /// in a regular expression.
    pub fn replacement(
        &self,
        text: &str,
        scope: Range<usize>,
        found: Range<usize>,
        with: &str,
    ) -> String {
        let mut out = String::new();
        let haystack = &text[scope.clone()];
        match self
            .regex
            .captures_at(haystack, found.start - scope.start)
            .filter(|_| self.options.regex)
        {
            Some(groups) => groups.expand(with, &mut out),
            None => out.push_str(with),
        }
        match self.options.preserve_case {
            true => preserve_case(&text[found], &out),
            false => out,
        }
    }
}

/// The word around `offset`: letters, digits and `_`.
pub fn word_at(text: &str, offset: usize) -> Range<usize> {
    let word = |c: &char| c.is_alphanumeric() || *c == '_';
    let start = offset - text[..offset].chars().rev().take_while(word).map(char::len_utf8).sum::<usize>();
    let end = offset + text[offset..].chars().take_while(word).map(char::len_utf8).sum::<usize>();
    start..end
}

/// `with` in the case of `found`: all upper, all lower, or capitalised.
fn preserve_case(found: &str, with: &str) -> String {
    let letters: Vec<char> = found.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.is_empty() {
        return with.to_owned();
    }
    if letters.iter().all(|c| c.is_uppercase()) && letters.len() > 1 {
        return with.to_uppercase();
    }
    if letters.iter().all(|c| c.is_lowercase()) {
        return with.to_lowercase();
    }
    if letters[0].is_uppercase() {
        let mut chars = with.chars();
        return chars
            .next()
            .map(|first| first.to_uppercase().chain(chars).collect())
            .unwrap_or_default();
    }
    with.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(query: &str, options: Options, text: &str) -> Vec<String> {
        let finder = Finder::new(query, options).unwrap().unwrap();
        finder
            .matches(text, 0..text.len())
            .into_iter()
            .map(|r| text[r].to_owned())
            .collect()
    }

    #[test]
    fn plain_whole_word_and_case() {
        let text = "Select name, rename FROM t; SELECT Name";
        let plain = Options::default();
        assert_eq!(found("name", plain, text), ["name", "name", "Name"]);
        let word = Options {
            whole_word: true,
            ..plain
        };
        assert_eq!(found("name", word, text), ["name", "Name"]);
        let case = Options {
            match_case: true,
            ..word
        };
        assert_eq!(found("name", case, text), ["name"]);
        assert_eq!(found("a.b", plain, "a.b axb"), ["a.b"]);
        assert_eq!(found("ÁRVÍZ", plain, "árvíz"), ["árvíz"]);
    }

    #[test]
    fn regular_expressions_and_their_groups() {
        let regex = Options {
            regex: true,
            ..Options::default()
        };
        let text = "id int,\nname varchar(80)";
        assert_eq!(found(r"^\w+", regex, text), ["id", "name"]);
        assert_eq!(
            Finder::new("(", regex).unwrap().err().unwrap(),
            "Unclosed group"
        );
        assert!(Finder::new("", regex).is_none());
        let finder = Finder::new(r"(\w+) (\w+)", regex).unwrap().unwrap();
        let first = finder.matches(text, 0..text.len())[0].clone();
        assert_eq!(
            finder.replacement(text, 0..text.len(), first, "$2 $1"),
            "int id"
        );
    }

    #[test]
    fn matches_stay_in_the_scope() {
        let text = "a a a a";
        let finder = Finder::new("a", Options::default()).unwrap().unwrap();
        assert_eq!(finder.matches(text, 2..5), [2..3, 4..5]);
    }

    #[test]
    fn words_at_the_caret() {
        assert_eq!(word_at("let árvíz_2 = 1", 6), 4..13);
        assert_eq!(word_at("a.b_c", 3), 2..5);
        assert_eq!(word_at("a  b", 2), 2..2);
    }

    #[test]
    fn replacements_keep_the_case_found() {
        assert_eq!(preserve_case("NAME", "title"), "TITLE");
        assert_eq!(preserve_case("name", "Title"), "title");
        assert_eq!(preserve_case("Name", "title"), "Title");
        assert_eq!(preserve_case("nAme", "title"), "title");
        assert_eq!(preserve_case("42", "x"), "x");
    }
}
