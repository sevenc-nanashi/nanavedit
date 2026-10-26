use super::{Command, MENUS};
use gpui::{Context, EventEmitter, Render, Task, Window, div};
use std::sync::OnceLock;

enum Item {
    Command(muda::MenuItem),
    Check(muda::CheckMenuItem),
}

impl Item {
    fn id(&self) -> &muda::MenuId {
        match self {
            Self::Command(item) => item.id(),
            Self::Check(item) => item.id(),
        }
    }

    fn set_enabled(&self, enabled: bool) {
        match self {
            Self::Command(item) => item.set_enabled(enabled),
            Self::Check(item) => item.set_enabled(enabled),
        }
    }
}

pub struct Menubar {
    items: Vec<(Command, Item)>,
    state: Option<(bool, bool)>,
    _events: Task<()>,
}

impl EventEmitter<Command> for Menubar {}

impl Menubar {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // The application has one editor; keep the process-wide callback across editor reopenings.
        static EVENTS: OnceLock<async_channel::Receiver<muda::MenuEvent>> = OnceLock::new();
        let events = EVENTS
            .get_or_init(|| {
                let (sender, receiver) = async_channel::unbounded();
                muda::MenuEvent::set_event_handler(Some(move |event| {
                    let _ = sender.try_send(event);
                }));
                receiver
            })
            .clone();
        let menu = muda::Menu::new();
        #[cfg(target_os = "macos")]
        {
            // AppKit reserves the first top-level menu for the application.
            let app = muda::Submenu::new("nanavedit", true);
            app.append_items(&[
                &muda::PredefinedMenuItem::hide(None),
                &muda::PredefinedMenuItem::hide_others(None),
                &muda::PredefinedMenuItem::show_all(None),
            ])
            .expect("application menu");
            menu.append(&app).expect("application submenu");
        }
        let mut items = Vec::new();
        for (label, commands) in MENUS {
            let submenu = muda::Submenu::new(*label, true);
            for command in *commands {
                let Some(command) = *command else {
                    submenu
                        .append(&muda::PredefinedMenuItem::separator())
                        .expect("separator");
                    continue;
                };
                let accelerator = command
                    .shortcut()
                    .map(|shortcut| shortcut.parse().expect("menu shortcut"));
                let item = if command == Command::ProjectSettings {
                    let item = muda::CheckMenuItem::new(command.label(), true, false, accelerator);
                    submenu.append(&item).expect("check menu item");
                    Item::Check(item)
                } else {
                    let item = muda::MenuItem::new(command.label(), true, accelerator);
                    submenu.append(&item).expect("menu item");
                    Item::Command(item)
                };
                items.push((command, item));
            }
            menu.append(&submenu).expect("submenu");
        }
        #[cfg(target_os = "windows")]
        let hwnd = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let RawWindowHandle::Win32(handle) =
                window.window_handle().expect("window handle").as_raw()
            else {
                unreachable!("Windows window handle");
            };
            handle.hwnd.get()
        };
        let task = cx.spawn_in(window, async move |this, cx| {
            // Attach outside a GPUI update: SetMenu may synchronously resize the window.
            #[cfg(target_os = "windows")]
            unsafe {
                menu.init_for_hwnd(hwnd)
                    .expect("ネイティブメニューを作成できませんでした");
            }
            #[cfg(target_os = "macos")]
            menu.init_for_nsapp();
            // muda detaches the native menu when this task's owner is dropped.
            let _menu = menu;
            while let Ok(event) = events.recv().await {
                if this
                    .update(cx, |menubar, cx| {
                        if let Some((command, _)) = menubar
                            .items
                            .iter()
                            .find(|(_, item)| item.id() == event.id())
                        {
                            cx.emit(*command);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            items,
            state: None,
            _events: task,
        }
    }

    pub fn set_state(
        &mut self,
        loading: bool,
        project_settings: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if self.state == Some((loading, project_settings)) {
            return;
        }
        for (command, item) in &self.items {
            item.set_enabled(command.enabled(loading));
            if let Item::Check(item) = item {
                item.set_checked(project_settings);
            }
        }
        self.state = Some((loading, project_settings));
    }
}

impl Render for Menubar {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl gpui::IntoElement {
        div()
    }
}
