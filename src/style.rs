use iced::{
    Background, Color, Theme,
    widget::{button, container},
};

use crate::SyncIntention;

#[derive(Debug, Clone, Copy)]
pub enum PaneStyle {
    Left,
    Right,
}

impl container::StyleSheet for PaneStyle {
    type Style = Theme;

    fn appearance(&self, _style: &Theme) -> container::Appearance {
        match self {
            PaneStyle::Left => container::Appearance {
                background: Some(Background::Color(Color::from_rgb(0.50, 0.50, 0.70))),
                border: iced::Border {
                    color: Color::BLACK,
                    width: 2.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            PaneStyle::Right => container::Appearance {
                background: Some(Background::Color(Color::from_rgb(0.80, 0.80, 0.70))),
                border: iced::Border {
                    color: Color::BLACK,
                    width: 2.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        }
    }
}

pub struct ButtonStyle {
    pub selected: bool,
    pub has_problem: bool,
    pub intention: SyncIntention,
}

impl ButtonStyle {
    fn bg_color(&self) -> Color {
        let coef = if self.selected { 0.7 } else { 1.0 };

        match self.intention {
            SyncIntention::KeepSync => Color::from_rgb(0.90 * coef, 0.90 * coef, 0.90 * coef),
            SyncIntention::DropSync => Color::from_rgb(0.80 * coef, 0.60 * coef, 0.60 * coef),
            SyncIntention::Unspecified => Color::from_rgb(0.20 * coef, 0.90 * coef, 0.90 * coef),
        }
    }

    fn text_color(&self) -> Color {
        if self.has_problem {
            return Color::from_rgb(0.90, 0.10, 0.10);
        } else {
            return Color::BLACK;
        }
    }
}

impl button::StyleSheet for ButtonStyle {
    type Style = Theme;

    fn active(&self, _style: &Theme) -> button::Appearance {
        button::Appearance {
            background: Some(Background::Color(self.bg_color())),
            text_color: self.text_color(),
            ..Default::default()
        }
    }
}

pub struct ActionPanelStyle {}
impl container::StyleSheet for ActionPanelStyle {
    type Style = Theme;
    fn appearance(&self, _style: &Theme) -> container::Appearance {
        container::Appearance {
            background: Some(Background::Color(Color::from_rgb(0.80, 0.40, 0.30))),
            border: iced::Border {
                color: Color::BLACK,
                width: 2.0,
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

pub struct InfoPanelStyle {}
impl container::StyleSheet for InfoPanelStyle {
    type Style = Theme;
    fn appearance(&self, _style: &Theme) -> container::Appearance {
        container::Appearance {
            background: Some(Background::Color(Color::from_rgb(0.60, 0.70, 0.10))),
            border: iced::Border {
                color: Color::BLACK,
                width: 2.0,
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

pub struct TreePanelStyle {}
impl container::StyleSheet for TreePanelStyle {
    type Style = Theme;
    fn appearance(&self, _style: &Theme) -> container::Appearance {
        container::Appearance {
            background: Some(Background::Color(Color::from_rgb(0.70, 0.70, 0.70))),
            border: iced::Border {
                color: Color::BLACK,
                width: 2.0,
                ..Default::default()
            },
            ..Default::default()
        }
    }
}
