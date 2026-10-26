use gpui::{Context, Window};
pub use gpui_kit::prelude::{ContextMenu, ContextMenuEvent, MenuItem};

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod native;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub use native::Menubar;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod in_window;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub use in_window::Menubar;

pub fn configure_platform() {
    // GPUI's composition-only HWND cannot paint a native menu bar; select its HWND swap chain.
    #[cfg(target_os = "windows")]
    unsafe {
        // Called before GPUI starts its threads; set_var is also thread-safe on Windows.
        std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "1");
    }
}

pub const HEIGHT: f32 = if cfg!(any(target_os = "windows", target_os = "macos")) {
    0.0
} else {
    36.0
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Open,
    Save,
    SaveAs,
    ProjectSettings,
    AppSettings,
}

impl Command {
    fn label(self) -> &'static str {
        match self {
            Self::Open => "開く…",
            Self::Save => "保存",
            Self::SaveAs => "別名保存…",
            Self::ProjectSettings => "プロジェクト設定",
            Self::AppSettings => "アプリ設定…",
        }
    }

    fn shortcut(self) -> Option<&'static str> {
        match self {
            Self::Open => Some("Ctrl+O"),
            Self::Save => Some("Ctrl+S"),
            Self::SaveAs => Some("Ctrl+Shift+S"),
            Self::AppSettings => Some("Ctrl+,"),
            Self::ProjectSettings => None,
        }
    }

    fn enabled(self, loading: bool) -> bool {
        self == Self::AppSettings || !loading
    }
}

const MENUS: &[(&str, &[Option<Command>])] = &[
    (
        "ファイル",
        &[
            Some(Command::Open),
            Some(Command::Save),
            Some(Command::SaveAs),
            None,
            Some(Command::ProjectSettings),
        ],
    ),
    ("設定", &[Some(Command::AppSettings)]),
];

pub fn context_menu(
    id: &'static str,
    window: &mut Window,
    cx: &mut Context<ContextMenu>,
) -> ContextMenu {
    use gpui_kit::prelude::ContextMenuPresentation;
    let presentation = if cfg!(any(target_os = "windows", target_os = "macos")) {
        ContextMenuPresentation::Native
    } else {
        ContextMenuPresentation::InWindow
    };
    ContextMenu::new(id, window, cx).presentation(presentation)
}
