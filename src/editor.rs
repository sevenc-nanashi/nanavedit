use gpui::{
    Context, CursorStyle, Div, FocusHandle, IntoElement, MouseButton, Point, Render, Window, div,
    prelude::*, px,
};
use gpui_kit::{
    semantics::{NodeSpec, Role, Semantic, SemanticCoordinator},
    theme::Theme,
};

const DIVIDER: f32 = 8.0;
const HEADER: f32 = 36.0;
const PADDING: f32 = 24.0;

#[cfg(target_os = "windows")]
const UI_FONT: &str = "Yu Gothic UI";
#[cfg(target_os = "macos")]
const UI_FONT: &str = "Hiragino Sans";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const UI_FONT: &str = "Noto Sans CJK JP";

#[derive(Clone, Copy)]
enum Panel {
    Settings,
    Timeline,
}

pub struct Editor {
    settings_width: f32,
    timeline_height: f32,
    focus: FocusHandle,
    settings_focus: FocusHandle,
    timeline_focus: FocusHandle,
    drag: Option<(Panel, Point<gpui::Pixels>)>,
}

impl Editor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            settings_width: 300.0,
            timeline_height: 240.0,
            focus,
            settings_focus: cx.focus_handle().tab_stop(true).tab_index(0),
            timeline_focus: cx.focus_handle().tab_stop(true).tab_index(1),
            drag: None,
        }
    }

    fn constrain(&mut self, window: &Window) {
        let viewport = window.viewport_size();
        self.settings_width = self.settings_width.clamp(
            220.0,
            (f32::from(viewport.width) - DIVIDER - 400.0).max(220.0),
        );
        self.timeline_height = self.timeline_height.clamp(
            160.0,
            (f32::from(viewport.height) - DIVIDER - 160.0).max(160.0),
        );
    }

    fn resize(&mut self, panel: Panel, delta: f32, window: &Window, cx: &mut Context<Self>) {
        match panel {
            Panel::Settings => self.settings_width += delta,
            Panel::Timeline => self.timeline_height += delta,
        }
        self.constrain(window);
        cx.notify();
    }

    fn divider(&self, panel: Panel, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let (id, label, focus, cursor, orientation) = match panel {
            Panel::Settings => (
                "settings-divider",
                "設定パネルの幅を変更",
                &self.settings_focus,
                CursorStyle::ResizeLeftRight,
                gpui::accesskit::Orientation::Vertical,
            ),
            Panel::Timeline => (
                "timeline-divider",
                "タイムラインの高さを変更",
                &self.timeline_focus,
                CursorStyle::ResizeUpDown,
                gpui::accesskit::Orientation::Horizontal,
            ),
        };

        // SplitPaneは比率を保持するため、ここでは固定ピクセル寸法の境界を使う。
        div()
            .id(id)
            .track_focus(focus)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor(cursor)
            .bg(theme.colors.canvas)
            .hover(|style| style.bg(theme.colors.hover))
            .focus(|style| style.bg(theme.colors.focus))
            .when(matches!(panel, Panel::Settings), |element| {
                element.w(px(DIVIDER)).h_full()
            })
            .when(matches!(panel, Panel::Timeline), |element| {
                element.h(px(DIVIDER)).w_full()
            })
            .child(
                div()
                    .bg(theme.colors.divider)
                    .when(matches!(panel, Panel::Settings), |element| {
                        element.w(px(2.0)).h(px(24.0))
                    })
                    .when(matches!(panel, Panel::Timeline), |element| {
                        element.h(px(2.0)).w(px(24.0))
                    }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |editor, event: &gpui::MouseDownEvent, window, cx| {
                    match panel {
                        Panel::Settings => editor.settings_focus.focus(window, cx),
                        Panel::Timeline => editor.timeline_focus.focus(window, cx),
                    }
                    editor.drag = Some((panel, event.position));
                    cx.notify();
                }),
            )
            .on_key_down(
                cx.listener(move |editor, event: &gpui::KeyDownEvent, window, cx| {
                    let delta = match (panel, event.keystroke.key.as_str()) {
                        (Panel::Settings, "left") | (Panel::Timeline, "up") => 10.0,
                        (Panel::Settings, "right") | (Panel::Timeline, "down") => -10.0,
                        _ => return,
                    };
                    editor.resize(panel, delta, window, cx);
                    cx.stop_propagation();
                }),
            )
            .semantic_in(
                cx,
                NodeSpec::new(id, Role::Splitter)
                    .text(label)
                    .orientation(orientation),
            )
    }
}

fn panel(title: &'static str, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .size_full()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .bg(theme.colors.panel)
        .child(
            div()
                .h(px(HEADER))
                .flex_none()
                .flex()
                .items_center()
                .px(px(16.0))
                .border_b_1()
                .border_color(theme.colors.hairline)
                .child(title),
        )
}

fn empty_message(message: &'static str, theme: &Theme) -> Div {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .items_center()
        .justify_center()
        .p(px(PADDING))
        .text_center()
        .text_color(theme.colors.text_muted)
        .child(message)
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        SemanticCoordinator::global(cx).begin_frame(window);
        self.constrain(window);
        let theme = Theme::get(cx).clone();
        let viewport = window.viewport_size();
        let preview_width = f32::from(viewport.width) - self.settings_width - DIVIDER;
        let top_height = f32::from(viewport.height) - self.timeline_height - DIVIDER;
        let frame_width = (preview_width - PADDING * 2.0)
            .min((top_height - HEADER - PADDING * 2.0) * 16.0 / 9.0)
            .max(0.0);

        div()
            .id("editor")
            .track_focus(&self.focus)
            .tab_group()
            .tab_stop(false)
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme.colors.canvas)
            .text_color(theme.colors.text)
            .font_family(UI_FONT)
            .text_size(px(13.0))
            .on_key_down(|event, window, cx| {
                if event.keystroke.key == "tab" {
                    if event.keystroke.modifiers.shift {
                        window.focus_prev(cx);
                    } else {
                        window.focus_next(cx);
                    }
                    cx.stop_propagation();
                }
            })
            .on_mouse_move(
                cx.listener(|editor, event: &gpui::MouseMoveEvent, window, cx| {
                    if let Some((panel, previous)) = editor.drag {
                        if event.pressed_button != Some(MouseButton::Left) {
                            editor.drag = None;
                            return;
                        }
                        let delta = match panel {
                            Panel::Settings => previous.x - event.position.x,
                            Panel::Timeline => previous.y - event.position.y,
                        };
                        editor.drag = Some((panel, event.position));
                        editor.resize(panel, delta.into(), window, cx);
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|editor, _, _, _| editor.drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|editor, _, _, _| editor.drag = None),
            )
            .child(
                div()
                    .flex()
                    .w_full()
                    .h(px(top_height))
                    .flex_none()
                    .child(
                        div().w(px(preview_width)).h_full().flex_none().child(
                            panel("プレビュー", &theme).child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(theme.colors.sunken)
                                    .child(
                                        div()
                                            .w(px(frame_width))
                                            .h(px(frame_width * 9.0 / 16.0))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .p(px(8.0))
                                            .bg(gpui::black())
                                            .text_center()
                                            .text_color(theme.colors.text_muted)
                                            .child("プロジェクトがありません"),
                                    ),
                            ),
                        ),
                    )
                    .child(self.divider(Panel::Settings, cx))
                    .child(
                        div().w(px(self.settings_width)).h_full().flex_none().child(
                            panel("設定", &theme)
                                .child(empty_message("オブジェクトが選択されていません", &theme)),
                        ),
                    ),
            )
            .child(self.divider(Panel::Timeline, cx))
            .child(
                div()
                    .w_full()
                    .h(px(self.timeline_height))
                    .flex_none()
                    .child(
                        panel("タイムライン", &theme)
                            .child(empty_message("オブジェクトがありません", &theme)),
                    ),
            )
    }
}
