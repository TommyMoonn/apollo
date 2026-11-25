//! Compact, keyboard-operable dropdown for a fixed set of choices.
use std::{cell::Cell, rc::Rc};

use gpui::{
    Anchor, Bounds, Context, EventEmitter, FocusHandle, Pixels, Render, Role, Window, anchored,
    deferred, div, point, prelude::*, px, rgba,
};

use super::{
    components::{self, ActionState, Button, Tone},
    design::{METRICS as M, Palette},
};

/// The consuming form owns the selected value; this event requests a change.
pub(super) struct Selected(pub usize);

pub(super) struct Dropdown {
    labels: &'static [&'static str],
    selected: usize,
    enabled: bool,
    palette: Palette,
    open: bool,
    focus: FocusHandle,
    options: Vec<FocusHandle>,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl EventEmitter<Selected> for Dropdown {}

impl Dropdown {
    pub fn new(labels: &'static [&'static str], selected: usize, cx: &mut Context<Self>) -> Self {
        assert!(selected < labels.len());
        Self {
            labels,
            selected,
            enabled: true,
            palette: Palette::DARK,
            open: false,
            focus: cx.focus_handle().tab_stop(true).tab_index(0),
            options: labels.iter().map(|_| cx.focus_handle()).collect(),
            bounds: Rc::new(Cell::new(None)),
        }
    }

    pub fn sync(
        &mut self,
        selected: usize,
        enabled: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) {
        if self.selected == selected && self.enabled == enabled && self.palette == palette {
            return;
        }
        self.selected = selected;
        self.enabled = enabled;
        self.palette = palette;
        if !enabled {
            self.open = false;
        }
        cx.notify();
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled {
            return;
        }
        if self.open {
            self.dismiss(window, cx);
        } else {
            self.open = true;
            window.focus(&self.options[self.selected], cx);
            cx.notify();
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled || !self.open || index >= self.labels.len() {
            return;
        }
        self.dismiss(window, cx);
        if index != self.selected {
            cx.emit(Selected(index));
        }
    }

    fn key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let stroke = &event.keystroke;
        if stroke.key == "tab" && self.open {
            // Return to the trigger before the shell advances its normal tab order.
            self.dismiss(window, cx);
            return;
        }
        if stroke.modifiers.modified() || !self.enabled {
            return;
        }
        if !self.open {
            if matches!(stroke.key.as_str(), "up" | "down") {
                self.toggle(window, cx);
                cx.stop_propagation();
            }
            return;
        }
        let focused = self
            .options
            .iter()
            .position(|focus| focus.is_focused(window))
            .unwrap_or(self.selected);
        let next = match stroke.key.as_str() {
            "escape" => {
                self.dismiss(window, cx);
                cx.stop_propagation();
                return;
            }
            "up" => focused.saturating_sub(1),
            "down" => (focused + 1).min(self.options.len() - 1),
            "home" => 0,
            "end" => self.options.len() - 1,
            _ => return,
        };
        window.focus(&self.options[next], cx);
        cx.stop_propagation();
    }
}

impl Render for Dropdown {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
        let open = self.open;
        let bounds = Rc::clone(&self.bounds);
        let mut trigger = Button::new("format-trigger", self.labels[self.selected], Tone::Standard);
        trigger.alternate_labels = self.labels;
        if self.enabled {
            trigger.state = ActionState::Enabled(Box::new(
                cx.listener(|view, _, window, cx| view.toggle(window, cx)),
            ));
        }
        let trigger = components::icon_action(trigger, "⌄", palette, window)
            .role(Role::ComboBox)
            .aria_label(format!("Output format: {}", self.labels[self.selected]))
            .aria_expanded(open)
            .when(self.enabled, |trigger| trigger.track_focus(&self.focus));
        let selector = div()
            .relative()
            .flex_shrink_0()
            .child(trigger)
            .on_children_prepainted({
                let bounds = Rc::clone(&bounds);
                move |children, window, cx| {
                    if let Some(trigger_bounds) = children.first().copied() {
                        let previous = bounds.replace(Some(trigger_bounds));
                        if open && previous != Some(trigger_bounds) {
                            window.defer(cx, |window, _| window.refresh());
                        }
                    }
                }
            })
            .id("format-selector")
            .on_key_down(cx.listener(Self::key_down));
        let Some(target) = bounds.get().filter(|_| open) else {
            return selector;
        };
        let menu = div()
            .id("format-menu")
            .role(Role::ListBox)
            .aria_label("Output formats")
            // The popup needs room for its own padding in addition to the widest option.
            .min_w(target.size.width)
            .flex()
            .flex_col()
            .gap(px(M.tight))
            .p(px(M.tight))
            .rounded(px(M.radius))
            .border(px(M.rule))
            .border_color(rgba(palette.border()))
            .bg(rgba(palette.background))
            .occlude()
            .on_mouse_down_out(cx.listener(
                move |view, event: &gpui::MouseDownEvent, window, cx| {
                    // A trigger click toggles the menu instead of dismissing and reopening it.
                    if !bounds
                        .get()
                        .is_some_and(|bounds| bounds.contains(&event.position))
                    {
                        // Restore focus before the outside click is dispatched; a clicked
                        // control can then take focus normally.
                        view.dismiss(window, cx);
                    }
                },
            ))
            .children(self.labels.iter().zip(&self.options).enumerate().map(
                |(index, (&label, focus))| {
                    let mut option = Button::new(("format-option", index), label, Tone::Standard);
                    let select = Box::new(
                        cx.listener(move |view, _, window, cx| view.choose(index, window, cx)),
                    );
                    option.state = if self.selected == index {
                        ActionState::Selected(select)
                    } else {
                        ActionState::Enabled(select)
                    };
                    option
                        .render(palette, window)
                        .role(Role::ListBoxOption)
                        .aria_selected(self.selected == index)
                        .w_full()
                        .track_focus(focus)
                        .tab_stop(false)
                },
            ));
        selector.child(
            deferred(
                anchored()
                    .attach(target, Anchor::BottomLeft)
                    .offset(point(px(0.), px(M.tight)))
                    .child(menu),
            )
            .priority(1),
        )
    }
}
