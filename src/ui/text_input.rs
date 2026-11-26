//! Single-line native text input for operation filenames and mixer gains. GPUI owns IME delivery.
use super::{
    audio_mix::{GAIN_SPEC, NumberSpec},
    design::{METRICS as M, Palette, TextRole},
};
use crate::audio_mix::model::TrackGain;
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, EventEmitter, FocusHandle, GlobalElementId, InspectorElementId,
    KeyDownEvent, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, Role, ShapedLine, SharedString, Style, TextRun, UTF16Selection, Window, div, fill,
    point, prelude::*, px, relative, rgba, size,
};
use std::ops::Range;

pub(super) struct Changed(pub String);

pub(super) struct TextInput {
    focus: FocusHandle,
    label: String,
    number: Option<(i16, NumberSpec)>,
    _number_blur: Option<gpui::Subscription>,
    content: SharedString,
    committed: SharedString,
    anchor: usize,
    cursor: usize,
    marked: Option<Range<usize>>,
    layout: Option<(ShapedLine, Bounds<Pixels>)>,
    scroll: Pixels,
    selecting: bool,
    enabled: bool,
    palette: Palette,
}
impl EventEmitter<Changed> for TextInput {}

impl TextInput {
    pub(super) fn new(value: String, cx: &mut Context<Self>) -> Self {
        let end = value.len();
        let content: SharedString = value.into();
        Self {
            focus: cx.focus_handle().tab_stop(true).tab_index(0),
            label: "Filename stem".to_owned(),
            number: None,
            _number_blur: None,
            committed: content.clone(),
            content,
            anchor: end,
            cursor: end,
            marked: None,
            layout: None,
            scroll: px(0.),
            selecting: false,
            enabled: true,
            palette: Palette::DARK,
        }
    }
    pub(super) fn new_gain(
        gain: TrackGain,
        label: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_number(gain.tenths_db(), label, GAIN_SPEC, window, cx)
    }
    pub(super) fn new_number(
        value: i16,
        label: String,
        spec: NumberSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut input = Self::new(number_text(value), cx);
        input.label = label;
        input.number = Some((value, spec));
        input._number_blur = Some(cx.on_blur(&input.focus, window, |input, window, cx| {
            input.selecting = false;
            input.restore_number();
            window.notify_text_input_changed(&input.focus);
            cx.notify();
        }));
        input
    }
    pub(super) fn sync_gain(
        &mut self,
        gain: TrackGain,
        enabled: bool,
        palette: Palette,
        window: &Window,
    ) {
        self.sync_number(gain.tenths_db(), enabled, palette, window);
    }
    pub(super) fn sync_number(
        &mut self,
        value: i16,
        enabled: bool,
        palette: Palette,
        window: &Window,
    ) {
        self.enabled = enabled;
        self.palette = palette;
        if let Some((previous, spec)) = self.number
            && (previous != value || !enabled || !self.focus.is_focused(window))
        {
            self.number = Some((value, spec));
            self.restore_number();
        }
    }
    fn restore_number(&mut self) {
        if let Some((number, _)) = self.number {
            let value = number_text(number);
            if self.content.as_ref() != value || self.marked.is_some() {
                self.content = value.into();
                self.cursor = self.content.len();
                self.anchor = self.cursor;
                self.marked = None;
                self.layout = None;
                self.scroll = px(0.);
            }
            self.committed = self.content.clone();
        }
    }
    pub(super) fn sync(&mut self, value: &str, enabled: bool, palette: Palette) {
        self.enabled = enabled;
        self.palette = palette;
        if self.committed.as_ref() != value {
            self.content = value.to_owned().into();
            self.committed = self.content.clone();
            self.cursor = self.content.len();
            self.anchor = self.cursor;
            self.marked = None;
            self.scroll = px(0.);
            self.layout = None;
        }
        if !enabled {
            self.selecting = false;
            if self.marked.take().is_some() {
                self.content = self.committed.clone();
                self.cursor = self.content.len();
                self.anchor = self.cursor;
            }
        }
    }
    pub(super) fn composing(&self) -> bool {
        self.marked.is_some()
    }
    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }
    fn utf16(&self, byte: usize) -> usize {
        self.content[..byte].encode_utf16().count()
    }
    fn byte(&self, units: usize) -> usize {
        let mut remaining = units;
        for (index, ch) in self.content.char_indices() {
            if remaining < ch.len_utf16() {
                return index;
            }
            remaining -= ch.len_utf16();
        }
        self.content.len()
    }
    fn range(&self, range: Range<usize>) -> Range<usize> {
        self.byte(range.start)..self.byte(range.end.max(range.start))
    }
    fn previous(&self) -> usize {
        self.content[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }
    fn next(&self) -> usize {
        self.cursor
            + self.content[self.cursor..]
                .chars()
                .next()
                .map_or(0, char::len_utf8)
    }
    fn select(&mut self, cursor: usize, extend: bool, window: &Window, cx: &mut Context<Self>) {
        self.cursor = cursor;
        if !extend {
            self.anchor = cursor;
        }
        window.notify_text_selection_changed(&self.focus);
        cx.notify();
    }
    fn publish(&mut self, window: &Window, cx: &mut Context<Self>) {
        if let Some((previous, spec)) = self.number {
            if let Some(value) = spec.parse(&self.content)
                && value != previous
            {
                self.number = Some((value, spec));
                cx.emit(Changed(number_text(value)));
            }
        } else if self.content != self.committed {
            self.committed = self.content.clone();
            cx.emit(Changed(self.content.to_string()));
        }
        window.notify_text_input_changed(&self.focus);
        cx.notify();
    }
    fn replace(&mut self, range: Range<usize>, text: &str) {
        let mut value = self.content.to_string();
        value.replace_range(range.clone(), text);
        self.content = value.into();
        self.cursor = range.start + text.len();
        self.anchor = self.cursor;
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled || self.composing() {
            return;
        }
        let key = &event.keystroke;
        let modifiers = key.modifiers;
        let selection = self.selection();
        if modifiers.control && !modifiers.alt && !modifiers.platform {
            match key.key.as_str() {
                "a" => {
                    self.anchor = 0;
                    self.select(self.content.len(), true, window, cx);
                }
                "c" | "x" => {
                    if !selection.is_empty() {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            self.content[selection.clone()].to_owned(),
                        ));
                        if key.key == "x" {
                            self.replace(selection, "");
                            self.publish(window, cx);
                        }
                    }
                }
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        if text.contains(['\n', '\r']) {
                            window.play_system_bell();
                            return;
                        }
                        // Keep invalid characters for domain validation, not silent sanitization.
                        self.replace(selection, &text);
                        self.publish(window, cx);
                    }
                }
                _ => return,
            }
        } else if !modifiers.control && !modifiers.alt && !modifiers.platform {
            match key.key.as_str() {
                "up" | "down" if self.number.is_some() => {
                    let (value, spec) = self.number.expect("numeric input");
                    let step = if key.key == "up" {
                        spec.step
                    } else {
                        -spec.step
                    };
                    let next = spec.step_value(value + step);
                    if next != value {
                        self.number = Some((next, spec));
                        cx.emit(Changed(number_text(next)));
                    }
                    self.restore_number();
                    window.notify_text_input_changed(&self.focus);
                    cx.notify();
                }
                "enter" | "escape" if self.number.is_some() => {
                    self.restore_number();
                    window.notify_text_input_changed(&self.focus);
                    cx.notify();
                }
                "left" => self.select(
                    if !modifiers.shift && !selection.is_empty() {
                        selection.start
                    } else {
                        self.previous()
                    },
                    modifiers.shift,
                    window,
                    cx,
                ),
                "right" => self.select(
                    if !modifiers.shift && !selection.is_empty() {
                        selection.end
                    } else {
                        self.next()
                    },
                    modifiers.shift,
                    window,
                    cx,
                ),
                "home" => self.select(0, modifiers.shift, window, cx),
                "end" => self.select(self.content.len(), modifiers.shift, window, cx),
                "backspace" | "delete" => {
                    let range = if !selection.is_empty() {
                        selection
                    } else if key.key == "backspace" {
                        self.previous()..self.cursor
                    } else {
                        self.cursor..self.next()
                    };
                    self.replace(range, "");
                    self.publish(window, cx);
                }
                _ => return,
            }
        } else {
            return;
        }
        cx.stop_propagation();
    }
    fn index_at(&self, point: Point<Pixels>) -> usize {
        self.layout
            .as_ref()
            .filter(|(line, _)| line.text == self.content)
            .map_or(self.cursor, |(line, bounds)| {
                line.closest_index_for_x(point.x - bounds.left())
            })
    }
    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled || self.composing() {
            return;
        }
        window.focus(&self.focus, cx);
        self.selecting = true;
        self.select(
            self.index_at(event.position),
            event.modifiers.shift,
            window,
            cx,
        );
    }
    fn mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            self.select(self.index_at(event.position), true, window, cx);
        }
    }
    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }
}
impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range(range);
        *actual = Some(self.utf16(range.start)..self.utf16(range.end));
        Some(self.content[range].to_owned())
    }
    fn selected_text_range(
        &mut self,
        ignore_disabled: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let range = self.selection();
        (self.enabled || ignore_disabled).then(|| UTF16Selection {
            range: self.utf16(range.start)..self.utf16(range.end),
            reversed: self.cursor < self.anchor,
        })
    }
    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.enabled {
            return;
        }
        let range = self.range(range);
        self.anchor = range.start;
        self.select(range.end, true, window, cx);
    }
    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.content.encode_utf16().count())
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|range| self.utf16(range.start)..self.utf16(range.end))
    }
    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.enabled {
            self.marked = None;
            self.publish(window, cx);
        }
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.enabled || text.contains(['\n', '\r']) {
            return;
        }
        let range = range
            .map(|range| self.range(range))
            .or(self.marked.take())
            .unwrap_or_else(|| self.selection());
        self.replace(range, text);
        self.marked = None;
        self.publish(window, cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.enabled || text.contains(['\n', '\r']) {
            return;
        }
        let range = range
            .map(|range| self.range(range))
            .or(self.marked.take())
            .unwrap_or_else(|| self.selection());
        let start = range.start;
        self.replace(range, text);
        self.marked = (!text.is_empty()).then_some(start..self.cursor);
        if let Some(selected) = selected {
            let offset = self.utf16(start);
            let len = text.encode_utf16().count();
            let selection =
                self.range(offset + selected.start.min(len)..offset + selected.end.min(len));
            self.anchor = selection.start;
            self.cursor = selection.end;
        }
        if self.marked.is_none() {
            self.publish(window, cx);
        } else {
            window.notify_text_input_changed(&self.focus);
            cx.notify();
        }
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (line, bounds) = self.layout.as_ref()?;
        if line.text != self.content {
            return None;
        }
        let range = self.range(range);
        Some(Bounds::from_corners(
            point(bounds.left() + line.x_for_index(range.start), bounds.top()),
            point(bounds.left() + line.x_for_index(range.end), bounds.bottom()),
        ))
    }
    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.utf16(self.index_at(point)))
    }
}

struct InputLine(Entity<TextInput>);
impl Element for InputLine {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let input = self.0.read(cx);
        let focus = input.focus.clone();
        let style = window.text_style();
        let line = window.text_system().shape_line(
            input.content.clone(),
            style.font_size.to_pixels(window.rem_size()),
            &[TextRun {
                len: input.content.len(),
                font: style.font(),
                color: style.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );
        let cursor = line.x_for_index(input.cursor);
        let scroll = input
            .scroll
            .min(cursor)
            .max(cursor - (bounds.size.width - px(2.)).max(px(0.)))
            .max(px(0.))
            .min((line.width() - bounds.size.width + px(2.)).max(px(0.)));
        let origin = point(bounds.left() - scroll, bounds.top());
        let selection = input.selection();
        let palette = input.palette;
        let marked = input.marked.clone();
        let enabled = input.enabled;
        if enabled {
            window.handle_input(&focus, ElementInputHandler::new(bounds, self.0.clone()), cx);
        }
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if !selection.is_empty() && focus.is_focused(window) {
                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(origin.x + line.x_for_index(selection.start), bounds.top()),
                        point(origin.x + line.x_for_index(selection.end), bounds.bottom()),
                    ),
                    rgba(palette.border()),
                ));
            }
            let _ = line.paint(
                origin,
                window.line_height(),
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            );
            if let Some(range) = marked {
                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(
                            origin.x + line.x_for_index(range.start),
                            bounds.bottom() - px(1.),
                        ),
                        point(origin.x + line.x_for_index(range.end), bounds.bottom()),
                    ),
                    rgba(palette.foreground),
                ));
            }
            if enabled && selection.is_empty() && focus.is_focused(window) {
                window.paint_quad(fill(
                    Bounds::new(
                        point(origin.x + cursor, bounds.top()),
                        size(px(1.), bounds.size.height),
                    ),
                    rgba(palette.foreground),
                ));
            }
        });
        self.0.update(cx, |input, _| {
            input.scroll = scroll;
            input.layout = Some((line, Bounds::new(origin, bounds.size)));
        });
    }
}
fn number_text(value: i16) -> String {
    format!("{:.1}", f64::from(value) / 10.)
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.number.is_some() && !self.focus.is_focused(window) {
            self.restore_number();
        }
        let palette = self.palette;
        TextRole::Control
            .apply(div())
            .id(if self.number.is_some() {
                "numeric-input"
            } else {
                "filename-stem"
            })
            .role(Role::TextInput)
            .aria_label(self.label.clone())
            .when_some(self.number, |element, (_, spec)| {
                element.aria_description(format!(
                    "{} to {} {}. Use Up and Down to adjust by {} or type a value.",
                    number_text(spec.minimum),
                    number_text(spec.maximum),
                    spec.unit,
                    number_text(spec.step)
                ))
            })
            .aria_value(self.content.clone())
            .flex()
            .items_center()
            .min_w_0()
            .w_full()
            .min_h(px(M.compact_height))
            .px(px(M.compact))
            .py(px(M.tight))
            .border(px(M.rule))
            .rounded(px(M.radius))
            .border_color(rgba(palette.border()))
            .text_color(rgba(
                if self.enabled
                    && self
                        .number
                        .is_none_or(|(_, spec)| spec.parse(&self.content).is_some())
                {
                    palette.foreground
                } else {
                    palette.disabled()
                },
            ))
            .when(self.enabled, |element| {
                element
                    .track_focus(&self.focus)
                    .tab_index(0)
                    .cursor(CursorStyle::IBeam)
                    .focus(move |style| style.border_color(rgba(palette.foreground)))
                    .on_key_down(cx.listener(Self::key))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    .on_mouse_move(cx.listener(Self::mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            })
            .child(InputLine(cx.entity()))
    }
}
