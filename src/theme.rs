use anyhow::{Result, ensure};
use gpui::{App, BorrowAppContext, Hsla};
use gpui_kit::{theme::ThemeRegistry, tokens};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HexColor(String);

impl TryFrom<String> for HexColor {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self> {
        let value = value.strip_prefix('#').unwrap_or(&value);
        ensure!(
            value.is_ascii() && value.len() == 6,
            "色は6桁の16進数で指定してください"
        );
        let value = format!("#{value}");
        tokens::Color::parse("Base16", &value)?;
        Ok(Self(value.to_ascii_lowercase()))
    }
}

impl From<HexColor> for String {
    fn from(value: HexColor) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Base16Palette {
    base00: HexColor,
    base01: HexColor,
    base02: HexColor,
    base03: HexColor,
    base04: HexColor,
    base05: HexColor,
    base06: HexColor,
    base07: HexColor,
    base08: HexColor,
    base09: HexColor,
    #[serde(rename = "base0A")]
    base0a: HexColor,
    #[serde(rename = "base0B")]
    base0b: HexColor,
    #[serde(rename = "base0C")]
    base0c: HexColor,
    #[serde(rename = "base0D")]
    base0d: HexColor,
    #[serde(rename = "base0E")]
    base0e: HexColor,
    #[serde(rename = "base0F")]
    base0f: HexColor,
}

impl Base16Palette {
    pub fn hex(&self, index: usize) -> &str {
        &[
            &self.base00,
            &self.base01,
            &self.base02,
            &self.base03,
            &self.base04,
            &self.base05,
            &self.base06,
            &self.base07,
            &self.base08,
            &self.base09,
            &self.base0a,
            &self.base0b,
            &self.base0c,
            &self.base0d,
            &self.base0e,
            &self.base0f,
        ][index]
            .0
    }

    fn color(&self, index: usize) -> tokens::Color {
        tokens::Color::parse("Base16", self.hex(index)).expect("validated Base16 color")
    }

    pub fn paint(&self, index: usize) -> Hsla {
        paint(self.color(index))
    }

    pub fn object(&self, index: usize, selected: bool) -> Hsla {
        paint(tokens::mix(
            self.color(0),
            self.color(index),
            if selected { 0.8 } else { 0.5 },
        ))
    }
}

fn paint(color: tokens::Color) -> Hsla {
    gpui::Rgba {
        r: color.red,
        g: color.green,
        b: color.blue,
        a: color.alpha,
    }
    .into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Base16Scheme {
    pub name: String,
    pub author: String,
    pub variant: tokens::Appearance,
    pub palette: Base16Palette,
}

#[derive(Deserialize)]
enum Base16System {
    #[serde(rename = "base16")]
    Base16,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SchemeFile {
    Current {
        system: Base16System,
        #[serde(flatten)]
        scheme: Base16Scheme,
    },
    Legacy {
        scheme: String,
        author: String,
        #[serde(flatten)]
        palette: Base16Palette,
    },
}

impl Base16Scheme {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        Ok(match serde_yaml_ng::from_str::<SchemeFile>(yaml)? {
            SchemeFile::Current {
                system: Base16System::Base16,
                scheme,
            } => scheme,
            SchemeFile::Legacy {
                scheme,
                author,
                palette,
            } => {
                let variant = if palette.color(0).luminance() < palette.color(5).luminance() {
                    tokens::Appearance::Dark
                } else {
                    tokens::Appearance::Light
                };
                Self {
                    name: scheme,
                    author,
                    variant,
                    palette,
                }
            }
        })
    }

    pub fn install(&self, cx: &mut App) {
        let mut document = match self.variant {
            tokens::Appearance::Dark => tokens::studio_dark().clone(),
            tokens::Appearance::Light => tokens::studio_light().clone(),
        };
        document.meta.id = "nanavedit-base16".into();
        document.meta.name.clone_from(&self.name);
        // shortcut: Disable scroll fading until wide primitives remain visible upstream.
        document.effect.edge_fade_band = 0.0;
        let b = |index| self.palette.hex(index).to_owned();
        let colors = &mut document.color;
        colors.surface = tokens::SurfaceColors {
            backdrop: b(0),
            canvas: b(0),
            sunken: b(0),
            panel: b(1),
            raised: b(2),
            overlay: b(1),
            scrim: b(0),
            control: b(1),
            control_hover: b(2),
            control_pressed: b(3),
        };
        colors.text = tokens::TextColors {
            primary: b(5),
            muted: b(4),
            faint: b(3),
            placeholder: b(4),
            disabled: b(3),
            on_accent: b(0),
            on_primary_fill: b(0),
        };
        colors.interactive = tokens::InteractiveColors {
            control_hairline: b(3),
            control_highlight: b(4),
            hover: b(2),
            active: b(3),
            selected: b(2),
            hairline: b(2),
            hairline_strong: b(3),
            track: b(2),
            divider: b(3),
            focus: b(13),
            primary_fill: b(13),
            white_fill: b(7),
            white_fill_hover: b(6),
            white_fill_active: b(5),
        };
        colors.semantic = tokens::SemanticColors {
            accent: b(13),
            accent_strong: b(12),
            danger: b(8),
            warning: b(10),
            success: b(11),
            info: b(13),
        };
        colors.loader = tokens::LoaderColors {
            mark: b(13),
            track: b(2),
            placeholder: b(1),
            sheen: b(3),
        };
        // Base16 authors own contrast; the kit's preset-specific contrast gates do not apply.
        cx.update_global::<ThemeRegistry, _>(|registry, _| registry.register(document));
        assert!(gpui_kit::theme::activate_theme("nanavedit-base16", cx));
    }
}

impl Default for Base16Scheme {
    fn default() -> Self {
        Self::from_yaml(include_str!("default-dark.yaml")).expect("bundled Base16 theme is valid")
    }
}
