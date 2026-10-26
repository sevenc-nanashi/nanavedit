use super::{Command, MENUS};
use gpui::{Context, Entity, EventEmitter, Render, Window, prelude::*};
use gpui_kit::prelude::{MenuItem, MenubarEvent, MenubarMenu};

pub struct Menubar {
    view: Entity<gpui_kit::prelude::Menubar>,
    state: Option<(bool, bool)>,
}

impl EventEmitter<Command> for Menubar {}

impl Menubar {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let view = cx.new(|cx| gpui_kit::prelude::Menubar::new("editor-menubar", [], window, cx));
        cx.subscribe(&view, |_, _, event, cx| {
            if let MenubarEvent::Invoked { item, .. } = event {
                let command = MENUS
                    .iter()
                    .flat_map(|(_, commands)| *commands)
                    .flatten()
                    .find(|command| command.label() == item.as_ref())
                    .expect("menu command");
                cx.emit(*command);
            }
        })
        .detach();
        Self { view, state: None }
    }

    pub fn set_state(
        &mut self,
        loading: bool,
        project_settings: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state == Some((loading, project_settings)) {
            return;
        }
        let menus = MENUS
            .iter()
            .map(|(label, commands)| {
                let items = commands.iter().map(|command| {
                    let Some(command) = command else {
                        return MenuItem::separator("separator");
                    };
                    let mut item = if *command == Command::ProjectSettings {
                        MenuItem::check(command.label(), command.label(), project_settings)
                    } else {
                        MenuItem::command(command.label(), command.label())
                    }
                    .disabled(!command.enabled(loading));
                    if let Some(shortcut) = command.shortcut() {
                        item = item.shortcut(shortcut);
                    }
                    item
                });
                MenubarMenu::new(*label, *label, items)
            })
            .collect::<Vec<_>>();
        self.view
            .update(cx, |view, cx| view.set_menus(menus, window, cx));
        self.state = Some((loading, project_settings));
    }
}

impl Render for Menubar {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl gpui::IntoElement {
        self.view.clone()
    }
}
