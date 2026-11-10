//! The Multi-Rename Tool (Ctrl+M): new names from masks as Total
//! Commander's — `[N]`, `[E]`, `[C]`, `[Y][M][D]`… — then search and
//! replace, then upper or lower case; checked, done and undone as a batch.

use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::time::SystemTime;

use regex::{Regex, RegexBuilder};

use crate::vfs::Fs;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Case {
    #[default]
    Unchanged,
    Lower,
    Upper,
    /// Only the first letter of the name.
    First,
    /// The first letter of every word.
    Words,
}

impl Case {
    pub const ALL: [Case; 5] = [
        Case::Unchanged,
        Case::Lower,
        Case::Upper,
        Case::First,
        Case::Words,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Case::Unchanged => "Unchanged",
            Case::Lower => "lowercase",
            Case::Upper => "UPPERCASE",
            Case::First => "First letter",
            Case::Words => "Every Word",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub name_mask: String,
    pub ext_mask: String,
    pub search: String,
    pub replace: String,
    pub case_sensitive: bool,
    /// Only the first match is replaced.
    pub first_only: bool,
    /// Search and replace sees the extension too.
    pub with_extension: bool,
    pub regex: bool,
    pub case: Case,
    pub counter_start: i64,
    pub counter_step: i64,
    pub counter_digits: usize,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            name_mask: "[N]".into(),
            ext_mask: "[E]".into(),
            search: String::new(),
            replace: String::new(),
            case_sensitive: false,
            first_only: false,
            with_extension: false,
            regex: false,
            case: Case::Unchanged,
            counter_start: 1,
            counter_step: 1,
            counter_digits: 1,
        }
    }
}

/// An entry to rename.
#[derive(Clone, Debug)]
pub struct Source {
    pub name: String,
    pub is_dir: bool,
    pub modified: Option<SystemTime>,
}

impl Source {
    /// Name and extension; folders keep their name whole.
    fn split(&self) -> (&str, &str) {
        match self.name.rfind('.') {
            Some(dot) if dot > 0 && !self.is_dir => (&self.name[..dot], &self.name[dot + 1..]),
            _ => (&self.name, ""),
        }
    }
}

/// The new names, in the order of `sources`; `folder` gives `[P]` and `[G]`.
pub fn preview(rules: &Rules, sources: &[Source], folder: &Path) -> Result<Vec<String>, String> {
    let search = match rules.search.is_empty() {
        true => None,
        false => {
            let pattern = match rules.regex {
                true => rules.search.clone(),
                false => regex::escape(&rules.search),
            };
            let built = RegexBuilder::new(&pattern)
                .case_insensitive(!rules.case_sensitive)
                .build()
                .map_err(|err| format!("The regular expression: {err}"))?;
            Some(built)
        }
    };
    let parent = name_of(folder);
    let grandparent = folder.parent().map(name_of).unwrap_or_default();
    Ok(sources
        .iter()
        .enumerate()
        .map(|(ix, source)| {
            let counter = rules.counter_start + rules.counter_step * ix as i64;
            let counter = format!("{counter:0width$}", width = rules.counter_digits);
            let fill = |mask: &str| expand(mask, source, &counter, &parent, &grandparent);
            let (name, ext) = (fill(&rules.name_mask), fill(&rules.ext_mask));
            let (name, ext) = match &search {
                None => (name, ext),
                Some(search) if rules.with_extension => {
                    let whole = join(&name, &ext);
                    let replaced = replace(search, &whole, &rules.replace, rules);
                    return change_case(&replaced, rules.case);
                }
                Some(search) => (replace(search, &name, &rules.replace, rules), ext),
            };
            change_case(&join(&name, &ext), rules.case)
        })
        .collect())
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn join(name: &str, ext: &str) -> String {
    match ext.is_empty() {
        true => name.to_owned(),
        false => format!("{name}.{ext}"),
    }
}

fn replace(search: &Regex, text: &str, with: &str, rules: &Rules) -> String {
    // A plain replacement has no `$1` to expand.
    let with = match rules.regex {
        true => with.to_owned(),
        false => with.replace('$', "$$"),
    };
    match rules.first_only {
        true => search.replace(text, with.as_str()).into_owned(),
        false => search.replace_all(text, with.as_str()).into_owned(),
    }
}

fn change_case(text: &str, case: Case) -> String {
    match case {
        Case::Unchanged => text.to_owned(),
        Case::Lower => text.to_lowercase(),
        Case::Upper => text.to_uppercase(),
        Case::First => {
            let mut chars = text.chars();
            match chars.next() {
                Some(first) => first
                    .to_uppercase()
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect(),
                None => String::new(),
            }
        }
        Case::Words => {
            let mut start = true;
            text.chars()
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
    }
}

/// A mask filled for one entry: `[N]`, `[N2-5]`, `[N3,2]`, `[N-3-]`,
/// `[E]`, `[C]`, `[P]`, `[G]`, `[Y]`, `[M]`, `[D]`, `[h]`, `[m]`, `[s]`;
/// `[[` is a `[`. Anything else stays as written.
fn expand(mask: &str, source: &Source, counter: &str, parent: &str, grandparent: &str) -> String {
    let (name, ext) = source.split();
    let time = |format: &str| {
        source
            .modified
            .map(|time| crate::local_time(time, format))
            .unwrap_or_default()
    };
    let mut out = String::new();
    let mut rest = mask;
    while let Some(open) = rest.find('[') {
        out += &rest[..open];
        rest = &rest[open..];
        if let Some(after) = rest.strip_prefix("[[") {
            out.push('[');
            rest = after;
            continue;
        }
        let Some(close) = rest.find(']') else {
            break;
        };
        let field = &rest[1..close];
        let filled = match field {
            "C" => Some(counter.to_owned()),
            "P" => Some(parent.to_owned()),
            "G" => Some(grandparent.to_owned()),
            "Y" => Some(time("%Y")),
            "M" => Some(time("%m")),
            "D" => Some(time("%d")),
            "h" => Some(time("%H")),
            "m" => Some(time("%M")),
            "s" => Some(time("%S")),
            _ => match field.split_at_checked(1) {
                Some(("N", range)) => part(name, range),
                Some(("E", range)) => part(ext, range),
                _ => None,
            },
        };
        match filled {
            Some(filled) => out += &filled,
            None => out += &rest[..=close],
        }
        rest = &rest[close + 1..];
    }
    out + rest
}

/// Characters of `text` by a range: `` all, `3` one, `2-5` from to,
/// `2-` to the end, `2,3` three from the second; negative counts from the end.
fn part(text: &str, range: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len() as i64;
    // 1-based, or from the end when negative.
    let at = |text: &str| -> Option<i64> {
        let n: i64 = text.parse().ok()?;
        Some(if n < 0 { len + n + 1 } else { n })
    };
    let (from, to) = if range.is_empty() {
        (1, len)
    } else if let Some((from, count)) = range.split_once(',') {
        let from = at(from)?;
        (from, from + count.parse::<i64>().ok()? - 1)
    } else if let Some(from) = range
        .strip_suffix('-')
        .filter(|from| !from.is_empty() && !from.ends_with('-'))
    {
        (at(from)?, len)
    } else {
        // `2-5`, `-3`, `-5--2`: the dash after the first number splits.
        let split = range[1..].find('-').map(|ix| ix + 1);
        match split {
            Some(dash) => (at(&range[..dash])?, at(&range[dash + 1..])?),
            None => {
                let one = at(range)?;
                (one, one)
            }
        }
    };
    let (from, to) = (from.max(1), to.min(len));
    Some(match from <= to {
        true => chars[(from - 1) as usize..to as usize].iter().collect(),
        false => String::new(),
    })
}

/// Why a new name cannot be: empty, with a slash, twice in the batch, or
/// taken by an entry the batch leaves alone. `taken` is the folder's names.
pub fn problems(old: &[String], new: &[String], taken: &HashSet<String>) -> Vec<Option<String>> {
    let leaving: HashSet<&String> = old.iter().collect();
    let mut seen = HashSet::new();
    let fold = |name: &str| match cfg!(any(windows, target_os = "macos")) {
        true => name.to_lowercase(),
        false => name.to_owned(),
    };
    let duplicates: HashSet<String> = new
        .iter()
        .filter(|name| !seen.insert(fold(name)))
        .map(|name| fold(name))
        .collect();
    old.iter()
        .zip(new)
        .map(|(old, new)| {
            Some(
                match () {
                    _ if new.trim().is_empty() => "no name",
                    _ if new.contains('/')
                        || (cfg!(windows)
                            && new.contains(['\\', ':', '*', '?', '"', '<', '>', '|'])) =>
                    {
                        "a character names cannot have"
                    }
                    _ if new == "." || new == ".." => "no name",
                    _ if duplicates.contains(&fold(new)) => "twice in the list",
                    _ if new != old && taken.contains(new) && !leaving.contains(new) => {
                        "exists already"
                    }
                    _ => return None,
                }
                .to_owned(),
            )
        })
        .collect()
}

/// Renames `pairs` (old, new) in `folder`, by way of temporary names when
/// one takes another's; gives back what it did, to undo it. Stops at the
/// first error, having undone the half-done.
pub fn rename_all(
    fs: &dyn Fs,
    folder: &Path,
    pairs: &[(String, String)],
) -> io::Result<Vec<(String, String)>> {
    let pairs: Vec<&(String, String)> = pairs.iter().filter(|(old, new)| old != new).collect();
    let olds: HashSet<&String> = pairs.iter().map(|(old, _)| old).collect();
    let crossing = pairs.iter().any(|(_, new)| olds.contains(new));
    let mut done: Vec<(String, String)> = Vec::new();
    let mut steps: Vec<(String, String)> = Vec::new();
    match crossing {
        false => steps.extend(pairs.iter().map(|(old, new)| (old.clone(), new.clone()))),
        true => {
            for (ix, (old, _)) in pairs.iter().enumerate() {
                steps.push((
                    old.clone(),
                    format!(".pc-rename-{}-{ix}", std::process::id()),
                ));
            }
            for (ix, (_, new)) in pairs.iter().enumerate() {
                steps.push((
                    format!(".pc-rename-{}-{ix}", std::process::id()),
                    new.clone(),
                ));
            }
        }
    }
    for (from, to) in steps {
        if let Err(err) = fs.rename(&folder.join(&from), &folder.join(&to)) {
            for (from, to) in done.iter().rev() {
                let _ = fs.rename(&folder.join(to), &folder.join(from));
            }
            return Err(io::Error::new(err.kind(), format!("{from}: {err}")));
        }
        done.push((from, to));
    }
    Ok(pairs
        .iter()
        .map(|(old, new)| (old.clone(), new.clone()))
        .collect())
}

/// What `rename_all` did, turned around.
pub fn undo_pairs(done: &[(String, String)]) -> Vec<(String, String)> {
    done.iter()
        .map(|(old, new)| (new.clone(), old.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::Local;

    fn source(name: &str) -> Source {
        Source {
            name: name.into(),
            is_dir: false,
            modified: crate::attributes::parse_time("2026-11-10 14:04:33"),
        }
    }

    fn names(rules: &Rules, list: &[&str]) -> Vec<String> {
        let sources: Vec<Source> = list.iter().map(|name| source(name)).collect();
        preview(rules, &sources, Path::new("/photos/2026/trip")).unwrap()
    }

    #[test]
    fn fills_masks() {
        let mut rules = Rules::default();
        assert_eq!(names(&rules, &["a.JPG"]), ["a.JPG"]);
        rules.name_mask = "[P]_[C]_[N2-4]".into();
        rules.counter_digits = 3;
        rules.counter_start = 9;
        assert_eq!(
            names(&rules, &["IMG1234.JPG", "IMG1235.JPG"]),
            ["trip_009_MG1.JPG", "trip_010_MG1.JPG"]
        );
        rules.name_mask = "[Y]-[M]-[D] [h][m][s] [N-3-] [N2,2] [G] [[x]".into();
        rules.ext_mask = "[E]".into();
        rules.case = Case::Lower;
        assert_eq!(
            names(&rules, &["IMG1234.JPG"]),
            ["2026-11-10 140433 234 mg 2026 [x].jpg"]
        );
        rules.name_mask = "[N][Q]".into();
        rules.case = Case::Words;
        assert_eq!(
            names(&rules, &["my holiday-photo.png"]),
            ["My Holiday-Photo[Q].Png"]
        );
    }

    #[test]
    fn searches_and_replaces() {
        let mut rules = Rules {
            search: "a".into(),
            replace: "$".into(),
            ..Rules::default()
        };
        assert_eq!(names(&rules, &["Banana.a"]), ["B$n$n$.a"]);
        rules.first_only = true;
        rules.with_extension = true;
        assert_eq!(names(&rules, &["Banana.a"]), ["B$nana.a"]);
        rules = Rules {
            search: r"(\d+)".into(),
            replace: "#$1".into(),
            regex: true,
            ..Rules::default()
        };
        assert_eq!(names(&rules, &["track7.mp3"]), ["track#7.mp3"]);
        rules.search = "(".into();
        let sources = [source("a")];
        assert!(preview(&rules, &sources, Path::new("/")).is_err());
    }

    #[test]
    fn finds_problems_and_renames_crosswise() {
        let old: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        let new: Vec<String> = ["b", "a", "x"].map(String::from).to_vec();
        let taken: HashSet<String> = ["a", "b", "c", "x"].map(String::from).into();
        let found = problems(&old, &new, &taken);
        assert_eq!(found, [None, None, Some("exists already".into())]);
        let twice = problems(&old, &["d".into(), "d".into(), "".into()], &taken);
        assert_eq!(twice[0].as_deref(), Some("twice in the list"));
        assert_eq!(twice[2].as_deref(), Some("no name"));

        let root = std::env::temp_dir().join(format!("pc-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a"), "A").unwrap();
        std::fs::write(root.join("b"), "B").unwrap();
        let pairs = [("a".to_string(), "b".to_string()), ("b".into(), "a".into())];
        let done = rename_all(&Local, &root, &pairs).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("a")).unwrap(), "B");
        rename_all(&Local, &root, &undo_pairs(&done)).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("a")).unwrap(), "A");
        assert_eq!(
            std::fs::read_dir(&root).unwrap().count(),
            2,
            "no temporary names left"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn takes_parts_of_names() {
        assert_eq!(part("abcdef", "").unwrap(), "abcdef");
        assert_eq!(part("abcdef", "2").unwrap(), "b");
        assert_eq!(part("abcdef", "2-4").unwrap(), "bcd");
        assert_eq!(part("abcdef", "2,3").unwrap(), "bcd");
        assert_eq!(part("abcdef", "3-").unwrap(), "cdef");
        assert_eq!(part("abcdef", "-2").unwrap(), "e");
        assert_eq!(part("abcdef", "-3-").unwrap(), "def");
        assert_eq!(part("abcdef", "-4--2").unwrap(), "cde");
        assert_eq!(part("abc", "5-9").unwrap(), "");
        assert!(part("abc", "x").is_none());
    }
}
