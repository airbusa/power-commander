//! A book's CSS, as far as the viewer's renderer can show it: what bolds,
//! slants, underlines or strikes text, what hides it and what puts it on a
//! line of its own (a poem's), for the selectors books mostly use, an element,
//! classes, an id (`p`, `.verse`, `p.verse`, `#title`), and `style`
//! attributes; other selectors are passed over.

use std::cell::RefCell;

use html5ever::serialize::{SerializeOpts, TraversalScope, serialize};
use html5ever::tendril::TendrilSink;
use html5ever::{Attribute, LocalName, QualName, namespace_url, ns};
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom, SerializableHandle};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Display {
    None,
    Block,
    Inline,
}

/// What a rule says that the renderer can show; none where it says nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Style {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    display: Option<Display>,
}

impl Style {
    /// `font-weight: bold; …`, as a rule's block or a `style` attribute has them.
    fn parse(declarations: &str) -> Style {
        let mut style = Style::default();
        for declaration in declarations.split(';') {
            let Some((name, value)) = declaration.split_once(':') else {
                continue;
            };
            let value = value.replace("!important", "").trim().to_lowercase();
            match name.trim().to_lowercase().as_str() {
                "font-weight" => {
                    let weight = match value.as_str() {
                        "bold" | "bolder" => 700,
                        "normal" | "lighter" => 400,
                        number => number.parse().unwrap_or(400),
                    };
                    style.bold = Some(weight >= 600);
                }
                "font-style" => style.italic = Some(matches!(value.as_str(), "italic" | "oblique")),
                "text-decoration" | "text-decoration-line" => {
                    style.underline = Some(value.contains("underline"));
                    style.strike = Some(value.contains("line-through"));
                }
                "display" => {
                    style.display = Some(match value.as_str() {
                        "none" => Display::None,
                        "inline" | "inline-block" => Display::Inline,
                        _ => Display::Block,
                    })
                }
                _ => {}
            }
        }
        style
    }

    /// `other` over this: what it says wins.
    fn cascade(&mut self, other: &Style) {
        self.bold = other.bold.or(self.bold);
        self.italic = other.italic.or(self.italic);
        self.underline = other.underline.or(self.underline);
        self.strike = other.strike.or(self.strike);
        self.display = other.display.or(self.display);
    }
}

/// `p`, `.verse`, `p.verse.first`, `#title`: one element's.
struct Selector {
    tag: Option<String>,
    classes: Vec<String>,
    id: Option<String>,
}

impl Selector {
    /// None for what it does not read: descendants, children, pseudo-classes, attributes.
    fn parse(text: &str) -> Option<Selector> {
        let text = text.trim();
        let name = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
        if text.is_empty() || !text.chars().all(|c| name(c) || c == '.' || c == '#') {
            return None;
        }
        let tag_end = text.find(['.', '#']).unwrap_or(text.len());
        let tag = Some(text[..tag_end].to_lowercase()).filter(|tag| !tag.is_empty() && tag != "*");
        let (mut classes, mut id) = (Vec::new(), None);
        let mut rest = &text[tag_end..];
        while let Some(mark) = rest.chars().next() {
            let end = rest[1..].find(['.', '#']).map_or(rest.len(), |at| at + 1);
            let part = rest[1..end].to_owned();
            if part.is_empty() {
                return None;
            }
            match mark {
                '.' => classes.push(part),
                _ => id = Some(part),
            }
            rest = &rest[end..];
        }
        Some(Selector { tag, classes, id })
    }

    /// Ids over classes over elements, as CSS weighs them.
    fn specificity(&self) -> (usize, usize, usize) {
        (self.id.is_some() as usize, self.classes.len(), self.tag.is_some() as usize)
    }

    fn matches(&self, tag: &str, classes: &[&str], id: Option<&str>) -> bool {
        self.tag.as_deref().is_none_or(|own| own == tag)
            && self.classes.iter().all(|class| classes.contains(&class.as_str()))
            && self.id.as_deref().is_none_or(|own| Some(own) == id)
    }
}

/// The book's rules, in the order its stylesheets have them.
#[derive(Default)]
pub(super) struct Stylesheet(Vec<(Selector, Style)>);

impl Stylesheet {
    pub(super) fn add(&mut self, css: &str) {
        let mut css = css.to_owned();
        while let Some(start) = css.find("/*") {
            let end = css[start..].find("*/").map_or(css.len(), |end| start + end + 2);
            css.replace_range(start..end, "");
        }
        let mut rest = css.as_str();
        while let Some(open) = rest.find('{') {
            let prelude = rest[..open].trim();
            // An at-rule's block is passed over whole, with what it nests.
            if prelude.starts_with('@') {
                let mut depth = 0;
                let end = rest[open..].char_indices().find_map(|(at, c)| {
                    depth += match c {
                        '{' => 1,
                        '}' => -1,
                        _ => 0,
                    };
                    (depth == 0).then_some(open + at + 1)
                });
                rest = &rest[end.unwrap_or(rest.len())..];
                continue;
            }
            let Some(close) = rest[open..].find('}') else {
                break;
            };
            let style = Style::parse(&rest[open + 1..open + close]);
            for selector in prelude.split(',').filter_map(Selector::parse) {
                self.0.push((selector, style));
            }
            rest = &rest[open + close + 1..];
        }
    }

    /// `html` with what its elements' styles say shown by elements the renderer has.
    pub(super) fn apply(&self, html: &str) -> String {
        let dom = html5ever::parse_document(RcDom::default(), Default::default()).one(html);
        let Some(body) = find(&dom.document, "body") else {
            return html.to_owned();
        };
        self.restyle(&body);
        let mut out = Vec::new();
        let opts = SerializeOpts { traversal_scope: TraversalScope::ChildrenOnly(None), ..Default::default() };
        match serialize(&mut out, &SerializableHandle::from(body), opts) {
            Ok(()) => String::from_utf8_lossy(&out).into_owned(),
            Err(_) => html.to_owned(),
        }
    }

    fn style_of(&self, tag: &str, attrs: &[Attribute]) -> Style {
        let attribute = |name: &str| attrs.iter().find(|a| &*a.name.local == name).map(|a| a.value.to_string());
        let class = attribute("class").unwrap_or_default();
        let classes: Vec<&str> = class.split_whitespace().collect();
        let id = attribute("id");
        let mut matching: Vec<(usize, &(Selector, Style))> = self
            .0
            .iter()
            .enumerate()
            .filter(|(_, (selector, _))| selector.matches(tag, &classes, id.as_deref()))
            .collect();
        matching.sort_by_key(|(order, (selector, _))| (selector.specificity(), *order));
        let mut style = Style::default();
        for (_, (_, rule)) in matching {
            style.cascade(rule);
        }
        if let Some(declarations) = attribute("style") {
            style.cascade(&Style::parse(&declarations));
        }
        style
    }

    fn restyle(&self, node: &Handle) {
        let children: Vec<Handle> = node.children.borrow().clone();
        let mut kept = Vec::with_capacity(children.len());
        for child in children {
            let NodeData::Element { name, attrs, .. } = &child.data else {
                kept.push(child);
                continue;
            };
            let tag = name.local.to_string();
            let style = self.style_of(&tag, &attrs.borrow());
            if style.display == Some(Display::None) {
                continue;
            }
            self.restyle(&child);
            // An inline element shown as a block, as a poem's lines are.
            let child = match style.display == Some(Display::Block) && INLINE.contains(&tag.as_str()) {
                true => renamed(&child, "div"),
                false => child,
            };
            let marks = [(style.bold, "b"), (style.italic, "i"), (style.underline, "u"), (style.strike, "s")];
            for (on, mark) in marks {
                if on == Some(true) && !implies(&tag, mark) {
                    wrap_children(&child, mark);
                }
            }
            kept.push(child);
        }
        *node.children.borrow_mut() = kept;
    }
}

/// Elements HTML shows in a line, which `display: block` puts on one of their own.
const INLINE: &[&str] = &["span", "a", "em", "i", "b", "strong", "small", "big", "font", "cite", "q", "u", "s"];

/// The element shows `mark` already: a heading is bold, `<em>` slanted.
fn implies(tag: &str, mark: &str) -> bool {
    match mark {
        "b" => matches!(tag, "b" | "strong" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "th"),
        "i" => matches!(tag, "i" | "em" | "cite"),
        "u" => tag == "u",
        _ => matches!(tag, "s" | "strike" | "del"),
    }
}

fn element(tag: &str, attrs: Vec<Attribute>) -> Handle {
    Node::new(NodeData::Element {
        name: QualName::new(None, ns!(html), LocalName::from(tag)),
        attrs: RefCell::new(attrs),
        template_contents: RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    })
}

/// Its children inside a `tag` of its own.
fn wrap_children(node: &Handle, tag: &str) {
    let wrapper = element(tag, Vec::new());
    *wrapper.children.borrow_mut() = std::mem::take(&mut *node.children.borrow_mut());
    node.children.borrow_mut().push(wrapper);
}

/// The same element, its attributes and children, as a `tag`.
fn renamed(node: &Handle, tag: &str) -> Handle {
    let NodeData::Element { attrs, .. } = &node.data else {
        return node.clone();
    };
    let new = element(tag, attrs.borrow().clone());
    *new.children.borrow_mut() = node.children.borrow().clone();
    new
}

fn find(node: &Handle, tag: &str) -> Option<Handle> {
    if let NodeData::Element { name, .. } = &node.data
        && &*name.local == tag
    {
        return Some(node.clone());
    }
    node.children.borrow().iter().find_map(|child| find(child, tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_what_the_renderer_can() {
        let mut sheet = Stylesheet::default();
        sheet.add(
            "/* a book's */ .verse { display: block } p.note { display: none }
             @media print { p { display: none } }
             #t { font-weight: bold } .i, em.x { font-style: italic }
             div p { font-weight: bold } p:first-child { display: none }",
        );
        let html = sheet.apply(
            r#"<p><span class="verse">One</span><span class="verse">Two</span></p><p class="note">hidden</p><p id="t">Title</p><p class="i">slanted</p><span style="text-decoration: underline">u</span><div><p>plain</p></div>"#,
        );
        assert!(html.contains(r#"<div class="verse">One</div><div class="verse">Two</div>"#), "{html}");
        assert!(!html.contains("hidden"), "display: none: {html}");
        assert!(html.contains(r#"<p id="t"><b>Title</b></p>"#), "{html}");
        assert!(html.contains(r#"<p class="i"><i>slanted</i></p>"#), "{html}");
        assert!(html.contains("<span style=\"text-decoration: underline\"><u>u</u></span>"), "{html}");
        assert!(html.contains("<p>plain</p>"), "descendants and @media passed over: {html}");
    }
}
