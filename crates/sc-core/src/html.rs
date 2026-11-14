//! HTML as text, as Lister's "HTML text" shows it.

/// Head, scripts and styles hidden; blocks on lines of their own, list items
/// as `  * `, links in `{ }`.
pub fn to_text(html: &str) -> String {
    let mut text = Text::default();
    let mut hidden: Option<String> = None;
    let mut rest = html;
    while !rest.is_empty() {
        if let Some(comment) = rest.strip_prefix("<!--") {
            rest = comment.split_once("-->").map_or("", |(_, after)| after);
            continue;
        }
        if let Some(tag) = rest.strip_prefix('<') {
            let (tag, after) = tag.split_once('>').unwrap_or((tag, ""));
            rest = after;
            let closing = tag.starts_with('/');
            let name: String = tag
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            if let Some(until) = &hidden {
                if closing && name == *until {
                    hidden = None;
                }
                continue;
            }
            text.tag(&name, closing, tag, &mut hidden);
            continue;
        }
        let end = rest.find('<').unwrap_or(rest.len());
        if hidden.is_none() {
            text.push_text(&entities(&rest[..end]));
        }
        rest = &rest[end..];
    }
    let end = text.out.trim_end().len();
    text.out.truncate(end);
    text.out
}

#[derive(Default)]
struct Text {
    out: String,
    space: bool,
    pre: bool,
    link: bool,
}

impl Text {
    fn tag(&mut self, name: &str, closing: bool, tag: &str, hidden: &mut Option<String>) {
        match (name, closing) {
            ("head" | "script" | "style" | "title", false) => *hidden = Some(name.to_owned()),
            ("br", _) => self.newline(),
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", true) => {
                self.line_start();
                self.newline();
            }
            ("li", false) => {
                self.line_start();
                self.out.push_str("  * ");
            }
            (
                "p" | "div" | "tr" | "table" | "ul" | "ol" | "blockquote" | "h1" | "h2" | "h3"
                | "h4" | "h5" | "h6",
                _,
            ) => self.line_start(),
            ("td" | "th", false) => self.out.push('\t'),
            ("pre", _) => {
                self.line_start();
                self.pre = !closing;
            }
            ("a", false) if tag.to_ascii_lowercase().contains("href") => {
                self.word_space();
                self.out.push('{');
                self.link = true;
            }
            ("a", true) if self.link => {
                self.out.push('}');
                self.link = false;
            }
            _ => {}
        }
    }

    fn push_text(&mut self, text: &str) {
        for c in text.chars() {
            if self.pre {
                self.out.push(c);
            } else if c.is_whitespace() {
                self.space = true;
            } else {
                self.word_space();
                self.out.push(c);
            }
        }
    }

    fn word_space(&mut self) {
        if self.space && !self.out.is_empty() && !self.out.ends_with(['\n', ' ', '\t', '{']) {
            self.out.push(' ');
        }
        self.space = false;
    }

    fn line_start(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        self.space = false;
    }

    fn newline(&mut self) {
        self.out.push('\n');
        self.space = false;
    }
}

fn entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let decoded = rest.find(';').filter(|&end| end < 12).and_then(|end| {
            let name = &rest[1..end];
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => ' ',
                _ => match name.strip_prefix('#') {
                    Some(hex) if hex.starts_with(['x', 'X']) => {
                        char::from_u32(u32::from_str_radix(&hex[1..], 16).ok()?)?
                    }
                    Some(decimal) => char::from_u32(decimal.parse().ok()?)?,
                    None => return None,
                },
            };
            Some((c, end + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_html_as_lister_does() {
        // The page Lister showed in the Windows VM.
        let page = "<html><head><title>Title</title><style>p{color:red}</style><script>var x=1;</script></head><body><h1>Heading</h1><p>First &amp; para<br>line two</p><ul><li>item one</li><li>item two</li></ul></body></html>";
        assert_eq!(
            to_text(page),
            "Heading\n\nFirst & para\nline two\n  * item one\n  * item two"
        );
        assert_eq!(
            to_text(
                "<p>See  <a href=x>the\n docs</a>, &#233;&#x41;&bogus;</p><!-- c --><pre>a  b\n c</pre>"
            ),
            "See {the docs}, éA&bogus;\na  b\n c"
        );
    }
}
