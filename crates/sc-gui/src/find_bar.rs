//! VS Code's find widget over the editor, in its top right corner (taken
//! over from Water Data Studio): find, and replace when expanded; by case,
//! whole word or regular expression; in the whole text or the selection only.

use std::ops::Range;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::input::{
    EditorState, Escape, Input, InputEvent, InputState, RangeDecoration, RangeDecorationCollection,
    RangeDecorationStyle,
};
use gpui_kit::component::{ActiveTheme, Disableable, Selectable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use sc_core::text_find::{self, Finder, LIMIT, Options, word_at};

actions!(
    find_bar,
    [
        ToggleMatchCase,
        ToggleWholeWord,
        ToggleRegex,
        ToggleInSelection,
        TogglePreserveCase,
        SelectAllMatches,
        ReplaceOne,
        ReplaceAll,
        OlderSearch,
        NewerSearch
    ]
);

const CONTEXT: &str = "FindBar";
const WIDTH: f32 = 460.;
/// The expand button and the gap after it.
const EXPAND_WIDTH: f32 = 26.;
/// Room for "? of 20000+".
const COUNT_WIDTH: f32 = 72.;
const HISTORY: usize = 50;

/// Bound in the bar's inputs: keys without a context rank as deep as the
/// focused input, so only these, bound later, come before them; and before
/// the inputs' own Up and Down.
pub fn bind_keys(cx: &mut App) {
    let keys = Some("FindBar > Input");
    cx.bind_keys([
        KeyBinding::new("alt-c", ToggleMatchCase, keys),
        KeyBinding::new("alt-w", ToggleWholeWord, keys),
        KeyBinding::new("alt-r", ToggleRegex, keys),
        KeyBinding::new("alt-l", ToggleInSelection, keys),
        KeyBinding::new("alt-p", TogglePreserveCase, keys),
        KeyBinding::new("alt-enter", SelectAllMatches, keys),
        KeyBinding::new("secondary-shift-1", ReplaceOne, keys),
        // Shift turns the 1 into an exclamation mark.
        KeyBinding::new("secondary-!", ReplaceOne, keys),
        KeyBinding::new("secondary-alt-enter", ReplaceAll, keys),
        KeyBinding::new("up", OlderSearch, keys),
        KeyBinding::new("down", NewerSearch, keys),
    ]);
}

/// VS Code's: the current match, the others, and the selection searched.
struct FindColors {
    current: Hsla,
    other: Hsla,
    scope: Hsla,
}

fn find_colors(dark: bool) -> FindColors {
    let (current, other, scope) = match dark {
        true => (0x27678290, 0x27678280, 0xFFFFFF13),
        false => (0x0069CC40, 0x0069CC1A, 0x00000015),
    };
    let rgba = |c: u32| Hsla::from(gpui_kit::rgba(c));
    FindColors { current: rgba(current), other: rgba(other), scope: rgba(scope) }
}

/// Searches of this run of the app, newest last.
#[derive(Default)]
struct History(Vec<String>);

impl Global for History {}

pub struct FindBar {
    editor: Entity<EditorState>,
    query: Entity<InputState>,
    replacement: Entity<InputState>,
    open: bool,
    expanded: bool,
    options: Options,
    /// The selection searched, while finding in it only.
    scope: Option<RangeDecorationCollection>,
    highlights: Option<RangeDecorationCollection>,
    matches: Vec<Range<usize>>,
    current: Option<usize>,
    error: Option<String>,
    /// Where typing a query searches from.
    origin: usize,
    /// The place in the history shown while browsing it.
    browsing: Option<usize>,
    _subscriptions: Vec<Subscription>,
}

impl FindBar {
    pub fn new(editor: Entity<EditorState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        let replacement = cx.new(|cx| InputState::new(window, cx).placeholder("Replace"));
        let subscriptions = vec![
            cx.subscribe_in(&query, window, |this, _, event, window, cx| match event {
                InputEvent::Change => {
                    this.browsing = None;
                    this.search(true, window, cx);
                }
                InputEvent::PressEnter { shift, .. } => match shift {
                    true => this.previous(window, cx),
                    false => this.next(window, cx),
                },
                _ => {}
            }),
            cx.subscribe_in(&replacement, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.replace_one(&ReplaceOne, window, cx);
                }
            }),
            cx.subscribe_in(&editor, window, |this, _, event, window, cx| {
                if this.open && matches!(event, InputEvent::Change) {
                    this.search(false, window, cx);
                }
            }),
        ];
        Self {
            editor,
            query,
            replacement,
            open: false,
            expanded: false,
            options: Options::default(),
            scope: None,
            highlights: None,
            matches: Vec::new(),
            current: None,
            error: None,
            origin: 0,
            browsing: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Ctrl+F and Ctrl+H: open, expand, or close.
    pub fn toggle(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        match (self.open, replace && !self.expanded) {
            (true, false) => self.close(window, cx),
            (true, true) => {
                self.expanded = true;
                self.replacement.update(cx, |r, cx| r.focus(window, cx));
                cx.notify();
            }
            (false, _) => self.open(replace, window, cx),
        }
    }

    fn open(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (text, selection) = self.text_and_selection(cx);
        self.origin = selection.start;
        self.expanded = replace;
        self.open = true;
        if text[selection.clone()].contains('\n') {
            self.set_scope(Some(selection), cx);
        } else {
            let seed = match selection.is_empty() {
                true => word_at(&text, selection.start),
                false => selection,
            };
            self.origin = seed.start;
            if !seed.is_empty() {
                let seed = match self.options.regex {
                    true => text_find::escape(&text[seed]),
                    false => text[seed].to_owned(),
                };
                self.query.update(cx, |q, cx| q.set_value(seed, window, cx));
            }
        }
        self.query.update(cx, |q, cx| {
            let len = q.value().len();
            q.set_selected_range(0..len, cx);
            q.focus(window, cx);
        });
        self.search(true, window, cx);
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remember(cx);
        self.open = false;
        self.set_scope(None, cx);
        self.matches.clear();
        self.current = None;
        if let Some(highlights) = &self.highlights {
            highlights.clear(cx);
        }
        self.editor.update(cx, |e, cx| e.focus(window, cx));
        cx.notify();
    }

    /// F3 and Shift+F3: the next or previous match, opening the bar first
    /// with the last search, or the word at the caret.
    pub fn find_again(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            match self.query.read(cx).value().is_empty() {
                true => self.open(false, window, cx),
                false => {
                    self.open = true;
                    self.expanded = false;
                    self.search(false, window, cx);
                }
            }
            self.editor.update(cx, |e, cx| e.focus(window, cx));
        }
        match forward {
            true => self.next(window, cx),
            false => self.previous(window, cx),
        }
    }

    /// Ctrl+F3: the next place of the word at the caret.
    pub fn find_word_at_caret(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (text, selection) = self.text_and_selection(cx);
        let range = match selection.is_empty() {
            true => word_at(&text, selection.start),
            false => selection,
        };
        if range.is_empty() || text[range.clone()].contains('\n') {
            return;
        }
        let word = match self.options.regex {
            true => text_find::escape(&text[range.clone()]),
            false => text[range.clone()].to_owned(),
        };
        // From the word itself, so the next one is found.
        self.editor
            .update(cx, |e, cx| e.set_selected_range(range, cx));
        if !self.open {
            self.open = true;
            self.expanded = false;
        }
        self.query.update(cx, |q, cx| q.set_value(word, window, cx));
        self.search(false, window, cx);
        self.next(window, cx);
    }

    fn text_and_selection(&self, cx: &App) -> (String, Range<usize>) {
        let editor = self.editor.read(cx);
        (editor.value().to_string(), editor.selected_range())
    }

    fn set_scope(&mut self, range: Option<Range<usize>>, cx: &mut Context<Self>) {
        let dark = cx.theme().mode.is_dark();
        match (range, &self.scope) {
            (None, Some(scope)) => {
                scope.clear(cx);
                self.scope = None;
            }
            (Some(range), _) => {
                let decoration = RangeDecoration::new(range)
                    .with_style(RangeDecorationStyle::Fill)
                    .with_color(find_colors(dark).scope);
                match &self.scope {
                    Some(scope) => scope.set(vec![decoration], cx),
                    None => {
                        let scope = self.editor.update(cx, |e, cx| {
                            e.create_range_decorations_collection(vec![decoration], cx)
                        });
                        self.scope = Some(scope);
                    }
                }
            }
            (None, None) => {}
        }
    }

    /// The searched range: the selection searched, else the whole script.
    fn scope_range(&self, len: usize, cx: &App) -> Range<usize> {
        match &self.scope {
            // Gone when all of it was deleted.
            Some(scope) => scope.get_ranges(cx).into_iter().next().unwrap_or(0..0),
            None => 0..len,
        }
    }

    /// Finds the matches again; `jump` selects the first one from where the
    /// search started, as typing a query does.
    fn search(&mut self, jump: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (text, selection) = self.text_and_selection(cx);
        let query = self.query.read(cx).value().to_string();
        let scope = self.scope_range(text.len(), cx);
        self.error = None;
        self.matches = match Finder::new(&query, self.options) {
            Some(Ok(finder)) => finder.matches(&text, scope),
            Some(Err(err)) => {
                self.error = Some(err);
                Vec::new()
            }
            None => Vec::new(),
        };
        self.current = self.matches.iter().position(|m| *m == selection);
        if jump && self.current.is_none() {
            let at = self
                .matches
                .iter()
                .position(|m| m.start >= self.origin)
                .or((!self.matches.is_empty()).then_some(0));
            if let Some(ix) = at {
                self.select(ix, window, cx);
                return;
            }
        }
        self.show(cx);
    }

    /// Highlights the matches.
    fn show(&mut self, cx: &mut Context<Self>) {
        let colors = find_colors(cx.theme().mode.is_dark());
        let decorations: Vec<RangeDecoration> = self
            .matches
            .iter()
            .enumerate()
            .map(|(ix, m)| {
                let color = match Some(ix) == self.current {
                    true => colors.current,
                    false => colors.other,
                };
                RangeDecoration::new(m.clone())
                    .with_style(RangeDecorationStyle::Fill)
                    .with_color(color)
            })
            .collect();
        match &self.highlights {
            Some(highlights) => highlights.set(decorations, cx),
            None => {
                let highlights = self.editor.update(cx, |e, cx| {
                    e.create_range_decorations_collection(decorations, cx)
                });
                self.highlights = Some(highlights);
            }
        }
        cx.notify();
    }

    fn select(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let _ = window;
        let Some(range) = self.matches.get(ix).cloned() else {
            return;
        };
        self.current = Some(ix);
        self.editor
            .update(cx, |e, cx| e.set_selected_range(range, cx));
        self.show(cx);
    }

    fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.editor.read(cx).selected_range();
        let ix = self
            .matches
            .iter()
            .position(|m| m.start >= selection.end)
            .unwrap_or(0);
        self.remember(cx);
        self.select(ix, window, cx);
    }

    fn previous(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.editor.read(cx).selected_range();
        let ix = self
            .matches
            .iter()
            .rposition(|m| m.start < selection.start)
            .unwrap_or(self.matches.len().saturating_sub(1));
        self.remember(cx);
        self.select(ix, window, cx);
    }

    fn toggle_option(
        &mut self,
        set: fn(&mut Options),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        set(&mut self.options);
        self.search(false, window, cx);
    }

    fn toggle_in_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.scope.is_some() {
            self.set_scope(None, cx);
        } else {
            let (text, mut selection) = self.text_and_selection(cx);
            if selection.is_empty() {
                // The caret's line.
                let start = text[..selection.start].rfind('\n').map_or(0, |ix| ix + 1);
                let end = text[selection.start..]
                    .find('\n')
                    .map_or(text.len(), |ix| selection.start + ix);
                selection = start..end;
            }
            self.set_scope(Some(selection), cx);
        }
        self.search(false, window, cx);
    }

    /// Ctrl+Shift+L: every match selected; with none, every place of the
    /// word at the caret, as VS Code's Select All Occurrences.
    pub fn select_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            self.find_word_at_caret(window, cx);
        }
        self.select_all_matches(&SelectAllMatches, window, cx);
    }

    /// Alt+Enter: every match selected, to edit them at once.
    fn select_all_matches(
        &mut self,
        _: &SelectAllMatches,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(first) = self.matches.first().cloned() else {
            return;
        };
        let rest = self.matches[1..].to_vec();
        self.editor.update(cx, |e, cx| {
            e.set_selected_range(first, cx);
            for range in rest {
                e.add_selection(range, cx);
            }
            e.focus(window, cx);
        });
    }

    /// Replaces the current match and moves on; with the selection not on a
    /// match, it only moves to the next one.
    fn replace_one(&mut self, _: &ReplaceOne, window: &mut Window, cx: &mut Context<Self>) {
        let (text, selection) = self.text_and_selection(cx);
        let query = self.query.read(cx).value().to_string();
        let Some(Ok(finder)) = Finder::new(&query, self.options) else {
            return;
        };
        if !self.matches.contains(&selection) {
            return self.next(window, cx);
        }
        let scope = self.scope_range(text.len(), cx);
        let with = self.replacement.read(cx).value().to_string();
        let replaced = finder.replacement(&text, scope, selection.clone(), &with);
        self.remember(cx);
        let after = selection.start + replaced.len();
        self.editor.update(cx, |e, cx| {
            e.set_selected_range(selection, cx);
            e.replace(replaced, window, cx);
            e.set_selected_range(after..after, cx);
        });
        self.search(false, window, cx);
        if !self.matches.is_empty() {
            self.next(window, cx);
        }
    }

    /// Every match replaced, as one change to undo.
    fn replace_all(&mut self, _: &ReplaceAll, window: &mut Window, cx: &mut Context<Self>) {
        let (text, _) = self.text_and_selection(cx);
        let query = self.query.read(cx).value().to_string();
        let Some(Ok(finder)) = Finder::new(&query, self.options) else {
            return;
        };
        let (Some(first), Some(last)) = (self.matches.first(), self.matches.last()) else {
            return;
        };
        let span = first.start..last.end;
        let scope = self.scope_range(text.len(), cx);
        let with = self.replacement.read(cx).value().to_string();
        let mut out = String::new();
        let mut at = span.start;
        for m in &self.matches {
            out.push_str(&text[at..m.start]);
            out.push_str(&finder.replacement(&text, scope.clone(), m.clone(), &with));
            at = m.end;
        }
        self.remember(cx);
        self.editor.update(cx, |e, cx| {
            e.set_selected_range(span, cx);
            e.replace(out, window, cx);
        });
        self.search(false, window, cx);
    }

    fn remember(&mut self, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value().to_string();
        if query.is_empty() {
            return;
        }
        let history = &mut cx.default_global::<History>().0;
        history.retain(|q| *q != query);
        history.push(query);
        if history.len() > HISTORY {
            history.remove(0);
        }
    }

    /// Up and Down in the query: earlier and later searches.
    fn browse(&mut self, older: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.query.focus_handle(cx).is_focused(window) {
            return cx.propagate();
        }
        let history = cx.default_global::<History>().0.clone();
        if history.is_empty() {
            return;
        }
        let ix = match (self.browsing, older) {
            (None, true) => history.len() - 1,
            (None, false) => return,
            (Some(ix), true) => ix.saturating_sub(1),
            (Some(ix), false) if ix + 1 < history.len() => ix + 1,
            (Some(_), false) => return,
        };
        let query = history[ix].clone();
        self.query
            .update(cx, |q, cx| q.set_value(query, window, cx));
        self.browsing = Some(ix);
        self.search(true, window, cx);
    }

    fn count(&self) -> String {
        match (self.matches.len(), self.current) {
            (0, _) => "No results".into(),
            (n, current) => {
                let total = match n >= LIMIT {
                    true => format!("{LIMIT}+"),
                    false => n.to_string(),
                };
                let at = current.map_or("?".to_owned(), |ix| (ix + 1).to_string());
                format!("{at} of {total}")
            }
        }
    }
}

fn toggle(
    id: &'static str,
    icon: Lucide,
    tooltip: &'static str,
    on: bool,
    cx: &Context<FindBar>,
    action: impl Fn(&mut FindBar, &mut Window, &mut Context<FindBar>) + 'static,
) -> Button {
    Button::new(id)
        .xsmall()
        .ghost()
        .icon(icon)
        .selected(on)
        .toggled(on)
        .tooltip(tooltip)
        .on_click(cx.listener(move |this, _, window, cx| action(this, window, cx)))
}

impl Render for FindBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let o = self.options;
        let has_matches = !self.matches.is_empty();
        let theme = cx.theme();
        let (border, popover, radius) = (theme.border, theme.popover, theme.radius);
        let danger = theme.danger;
        let count_color = match has_matches {
            true => theme.foreground,
            false => theme.muted_foreground,
        };
        let query_toggles = h_flex()
            .gap_0p5()
            .child(toggle(
                "find-case",
                Lucide::CaseSensitive,
                "Match Case (Alt+C)",
                o.match_case,
                cx,
                |this, w, cx| this.toggle_option(|o| o.match_case = !o.match_case, w, cx),
            ))
            .child(toggle(
                "find-word",
                Lucide::WholeWord,
                "Match Whole Word (Alt+W)",
                o.whole_word,
                cx,
                |this, w, cx| this.toggle_option(|o| o.whole_word = !o.whole_word, w, cx),
            ))
            .child(toggle(
                "find-regex",
                Lucide::Regex,
                "Use Regular Expression (Alt+R)",
                o.regex,
                cx,
                |this, w, cx| this.toggle_option(|o| o.regex = !o.regex, w, cx),
            ));
        let find_row = h_flex()
            .gap_1()
            .items_center()
            .child(
                Button::new("find-expand")
                    .xsmall()
                    .ghost()
                    .icon(match self.expanded {
                        true => Lucide::ChevronDown,
                        false => Lucide::ChevronRight,
                    })
                    .tooltip("Toggle Replace")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.expanded = !this.expanded;
                        cx.notify();
                    })),
            )
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.query)
                        .small()
                        .suffix(query_toggles)
                        .when(self.error.is_some(), |input| {
                            input.focus_bordered(false).border_color(danger)
                        }),
                ),
            )
            .child(
                div()
                    .w(px(COUNT_WIDTH))
                    .text_xs()
                    .text_color(count_color)
                    .whitespace_nowrap()
                    .child(self.count()),
            )
            .child(
                Button::new("find-previous")
                    .xsmall()
                    .ghost()
                    .icon(Lucide::ChevronUp)
                    .disabled(!has_matches)
                    .tooltip("Previous Match (Shift+Enter)")
                    .on_click(cx.listener(|this, _, window, cx| this.previous(window, cx))),
            )
            .child(
                Button::new("find-next")
                    .xsmall()
                    .ghost()
                    .icon(Lucide::ChevronDown)
                    .disabled(!has_matches)
                    .tooltip("Next Match (Enter)")
                    .on_click(cx.listener(|this, _, window, cx| this.next(window, cx))),
            )
            .child(toggle(
                "find-in-selection",
                Lucide::SquareDashedText,
                "Find in Selection (Alt+L)",
                self.scope.is_some(),
                cx,
                |this, w, cx| this.toggle_in_selection(w, cx),
            ))
            .child(
                Button::new("find-close")
                    .xsmall()
                    .ghost()
                    .icon(Lucide::X)
                    .tooltip("Close (Escape)")
                    .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
            );
        let replace_row =
            h_flex()
                .gap_1()
                .items_center()
                .child(div().w(px(EXPAND_WIDTH - 4.)).flex_none())
                .child(div().flex_1().min_w_0().child(
                    Input::new(&self.replacement).small().suffix(toggle(
                        "find-preserve-case",
                        Lucide::CaseUpper,
                        "Preserve Case (Alt+P)",
                        o.preserve_case,
                        cx,
                        |this, w, cx| {
                            this.toggle_option(|o| o.preserve_case = !o.preserve_case, w, cx)
                        },
                    )),
                ))
                .child(
                    Button::new("find-replace")
                        .xsmall()
                        .ghost()
                        .icon(Lucide::Replace)
                        .disabled(!has_matches)
                        .tooltip("Replace (Ctrl+Shift+1)")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.replace_one(&ReplaceOne, window, cx)
                        })),
                )
                .child(
                    Button::new("find-replace-all")
                        .xsmall()
                        .ghost()
                        .icon(Lucide::ReplaceAll)
                        .disabled(!has_matches)
                        .tooltip("Replace All (Ctrl+Alt+Enter)")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.replace_all(&ReplaceAll, window, cx)
                        })),
                )
                .child(div().w(px(COUNT_WIDTH + 4. * 24.)).flex_none());
        v_flex()
            .id("find-bar")
            .key_context(CONTEXT)
            .occlude()
            .w(px(WIDTH))
            .p_1()
            .gap_1()
            .bg(popover)
            .border_1()
            .border_color(border)
            .rounded(radius)
            .shadow_md()
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                this.close(window, cx);
                cx.stop_propagation();
            }))
            .on_action(
                cx.listener(|this, _: &OlderSearch, window, cx| this.browse(true, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &NewerSearch, window, cx| this.browse(false, window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleMatchCase, w, cx| {
                this.toggle_option(|o| o.match_case = !o.match_case, w, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleWholeWord, w, cx| {
                this.toggle_option(|o| o.whole_word = !o.whole_word, w, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleRegex, w, cx| {
                this.toggle_option(|o| o.regex = !o.regex, w, cx)
            }))
            .on_action(cx.listener(|this, _: &TogglePreserveCase, w, cx| {
                this.toggle_option(|o| o.preserve_case = !o.preserve_case, w, cx)
            }))
            .on_action(
                cx.listener(|this, _: &ToggleInSelection, w, cx| this.toggle_in_selection(w, cx)),
            )
            .on_action(cx.listener(Self::select_all_matches))
            .on_action(cx.listener(Self::replace_one))
            .on_action(cx.listener(Self::replace_all))
            .child(find_row)
            .when_some(self.error.clone(), |el, err| {
                el.child(
                    div()
                        .pl(px(EXPAND_WIDTH))
                        .text_xs()
                        .text_color(danger)
                        .child(err),
                )
            })
            .when(self.expanded, |el| el.child(replace_row))
    }
}
