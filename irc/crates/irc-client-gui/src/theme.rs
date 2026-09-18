use iced::color;
use iced::widget::container;
use iced::{Border, Color, Theme};

/// Theme choice persisted across the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ThemeChoice {
    Light,
    #[default]
    Dark,
}

impl ThemeChoice {
    /// Toggle between light and dark.
    #[must_use]
    pub(crate) fn toggle(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }

    /// Human-readable label for the current theme.
    #[must_use]
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    /// Resolve to an iced [`Theme`].
    #[must_use]
    pub(crate) fn to_iced(self) -> Theme {
        match self {
            Self::Light => mirc_classic(),
            Self::Dark => dark(),
        }
    }
}

/// Classic mIRC-inspired palette: white background, black text.
fn mirc_classic() -> Theme {
    Theme::custom(
        "mIRC Classic".into(),
        iced::theme::Palette {
            background: Color::WHITE,
            text: Color::BLACK,
            primary: color!(0x00, 0x3E, 0x7E),
            success: color!(0x00, 0x80, 0x00),
            danger: color!(0xCC, 0x00, 0x00),
        },
    )
}

/// Dark palette: dark background, light text.
#[allow(clippy::cognitive_complexity)]
fn dark() -> Theme {
    Theme::custom(
        "IRC Dark".into(),
        iced::theme::Palette {
            background: color!(0x1E, 0x1E, 0x2E),
            text: color!(0xCD, 0xD6, 0xF4),
            primary: color!(0x89, 0xB4, 0xFA),
            success: color!(0xA6, 0xE3, 0xA1),
            danger: color!(0xF3, 0x8B, 0xA8),
        },
    )
}

// ---------------------------------------------------------------------------
// Theme-aware container styles
// ---------------------------------------------------------------------------

/// Sidebar style (treebar, nicklist) — adapts to the active theme.
#[allow(clippy::cognitive_complexity)]
pub(crate) fn sidebar(theme: &Theme) -> container::Style {
    let is_dark = theme.extended_palette().is_dark;
    if is_dark {
        container::Style {
            background: Some(iced::Background::Color(color!(0x18, 0x18, 0x28))),
            text_color: Some(color!(0xCD, 0xD6, 0xF4)),
            border: Border {
                color: color!(0x31, 0x31, 0x44),
                width: 1.0,
                radius: 0.0.into(),
            },
            shadow: iced::Shadow::default(),
        }
    } else {
        container::Style {
            background: Some(iced::Background::Color(color!(0xF0, 0xF0, 0xF0))),
            text_color: Some(Color::BLACK),
            border: Border {
                color: color!(0xC0, 0xC0, 0xC0),
                width: 1.0,
                radius: 0.0.into(),
            },
            shadow: iced::Shadow::default(),
        }
    }
}

/// Topic bar / status bar style — adapts to the active theme.
#[allow(clippy::cognitive_complexity)]
pub(crate) fn topic_bar(theme: &Theme) -> container::Style {
    let is_dark = theme.extended_palette().is_dark;
    if is_dark {
        container::Style {
            background: Some(iced::Background::Color(color!(0x24, 0x24, 0x3E))),
            text_color: Some(color!(0xBA, 0xC2, 0xDE)),
            border: Border {
                color: color!(0x31, 0x31, 0x44),
                width: 1.0,
                radius: 0.0.into(),
            },
            shadow: iced::Shadow::default(),
        }
    } else {
        container::Style {
            background: Some(iced::Background::Color(color!(0xE8, 0xE8, 0xE8))),
            text_color: Some(color!(0x33, 0x33, 0x33)),
            border: Border {
                color: color!(0xC0, 0xC0, 0xC0),
                width: 1.0,
                radius: 0.0.into(),
            },
            shadow: iced::Shadow::default(),
        }
    }
}
