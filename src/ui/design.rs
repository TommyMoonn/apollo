//! Semantic roles for GPUI. The retained egui views still use `theme` and `fonts`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub background: u32,
    pub foreground: u32,
}

impl Palette {
    pub const DARK: Self = Self {
        background: 0x000000ff,
        foreground: 0xffffffff,
    };
    pub const LIGHT: Self = Self {
        background: 0xffffffff,
        foreground: 0x000000ff,
    };

    const fn ink(self, alpha: u8) -> u32 {
        (self.foreground & 0xffffff00) | alpha as u32
    }

    pub const fn secondary(self) -> u32 {
        self.ink(168)
    }
    pub const fn faint(self) -> u32 {
        self.ink(122)
    }
    pub const fn separator(self) -> u32 {
        self.ink(46)
    }
    pub const fn border(self) -> u32 {
        self.ink(77)
    }
    pub const fn hover(self) -> u32 {
        self.ink(14)
    }
    pub const fn pressed(self) -> u32 {
        self.ink(28)
    }
    pub const fn disabled(self) -> u32 {
        self.ink(87)
    }
    pub const fn primary_hover(self) -> u32 {
        self.ink(224)
    }
    pub const fn primary_pressed(self) -> u32 {
        self.ink(194)
    }
}

pub const DESTRUCTIVE_BORDER: u32 = 0x7e3030ff;
pub const FONT_FAMILY: &str = "Geist";
pub const WORDMARK_FAMILY: &str = "Fira Code";

/// Default typography roles at 100% UI zoom. Native chrome is OS-owned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextRole {
    Wordmark,
    PageTitle,
    SectionHeading,
    Body,
    Control,
    Status,
    Secondary,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Typography {
    pub family: &'static str,
    pub weight: u16,
    pub size: f32,
    pub line_height: f32,
    pub tabular_numbers: bool,
}

impl TextRole {
    pub const fn resolve(self) -> Typography {
        let (family, weight, size, line_height, tabular_numbers) = match self {
            Self::Wordmark => (WORDMARK_FAMILY, 400, 26., 32., false),
            Self::PageTitle => (FONT_FAMILY, 600, 20., 26., false),
            Self::SectionHeading => (FONT_FAMILY, 600, 14., 20., false),
            Self::Body => (FONT_FAMILY, 400, 14., 20., false),
            Self::Control | Self::Status => (FONT_FAMILY, 500, 14., 20., true),
            Self::Secondary => (FONT_FAMILY, 400, 12., 16., false),
        };
        Typography {
            family,
            weight,
            size,
            line_height,
            tabular_numbers,
        }
    }

    #[cfg(windows)]
    pub fn apply<E: gpui::Styled>(self, element: E) -> E {
        let typography = self.resolve();
        element
            .font(typography.font())
            .text_size(gpui::px(typography.size))
            .line_height(gpui::px(typography.line_height))
    }
}

#[cfg(windows)]
impl Typography {
    /// Use the same requested face and fallback chain for shaping and painting.
    pub fn font(self) -> gpui::Font {
        let mut font = gpui::font(self.family);
        font.weight = gpui::FontWeight(f32::from(self.weight));
        if self.tabular_numbers {
            // Keep counters stable without changing the typeface or shaping digits ourselves.
            static TABULAR: std::sync::LazyLock<gpui::FontFeatures> =
                std::sync::LazyLock::new(|| {
                    gpui::FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]))
                });
            font.features = TABULAR.clone();
        }
        font.fallbacks = Some(gpui::FontFallbacks::from_fonts(vec![
            "Segoe UI".into(),
            "Segoe UI Symbol".into(),
        ]));
        font
    }
}

pub const SPINNER_CYCLE_MS: u64 = 900;

/// Logical pixels; GPUI handles display scaling. Text-scale preferences arrive in U07.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub inset: f32,
    pub source_identity_min_width: f32,
    pub tight: f32,
    pub compact: f32,
    pub related: f32,
    pub focal: f32,
    pub section: f32,
    pub spacious: f32,
    pub action_inset: f32,
    pub action_gap: f32,
    pub primary_height: f32,
    pub compact_height: f32,
    pub row_height: f32,
    pub radius: f32,
    pub rule: f32,
    pub focus_width: f32,
    pub focus_offset: f32,
    pub icon_size: f32,
}

pub const METRICS: Metrics = Metrics {
    inset: 28.,
    source_identity_min_width: 220.,
    tight: 4.,
    compact: 7.,
    related: 11.,
    focal: 18.,
    section: 24.,
    spacious: 40.,
    action_inset: 10.,
    action_gap: 8.,
    primary_height: 31.,
    compact_height: 30.,
    row_height: 34.,
    radius: 4.,
    rule: 1.,
    focus_width: 2.,
    focus_offset: 2.,
    icon_size: 14.,
};

pub const SURFACE_MOTION_MS: u64 = 120;
pub const DETAIL_MOTION_MS: u64 = 110;
pub const HOVER_MOTION_MS: u64 = 85;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typography_resolves_the_default_role_contract() {
        for (role, family, weight, size, line_height, tabular_numbers) in [
            (TextRole::Wordmark, "Fira Code", 400, 26., 32., false),
            (TextRole::PageTitle, "Geist", 600, 20., 26., false),
            (TextRole::SectionHeading, "Geist", 600, 14., 20., false),
            (TextRole::Body, "Geist", 400, 14., 20., false),
            (TextRole::Control, "Geist", 500, 14., 20., true),
            (TextRole::Status, "Geist", 500, 14., 20., true),
            (TextRole::Secondary, "Geist", 400, 12., 16., false),
        ] {
            assert_eq!(
                role.resolve(),
                Typography {
                    family,
                    weight,
                    size,
                    line_height,
                    tabular_numbers,
                }
            );
        }
    }

    fn luminance(value: f64) -> f64 {
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }

    fn composite(ink: u32, background: u32) -> f64 {
        let alpha = f64::from(ink & 255) / 255.;
        let gray = |color: u32| f64::from((color >> 24) & 255) / 255.;
        luminance(gray(ink) * alpha + gray(background) * (1. - alpha))
    }

    #[test]
    fn essential_labels_keep_normal_text_contrast_in_both_palettes() {
        for palette in [Palette::DARK, Palette::LIGHT] {
            let background = composite(palette.background, palette.background);
            let contrast = |a: f64, b: f64| (a.max(b) + 0.05) / (a.min(b) + 0.05);
            assert!(
                contrast(
                    composite(palette.secondary(), palette.background),
                    background
                ) >= 4.5
            );
            for fill in [palette.background, palette.hover(), palette.pressed()] {
                assert!(
                    contrast(
                        composite(palette.foreground, palette.background),
                        composite(fill, palette.background)
                    ) >= 4.5
                );
            }
            for fill in [
                palette.foreground,
                palette.primary_hover(),
                palette.primary_pressed(),
            ] {
                assert!(contrast(composite(fill, palette.background), background) >= 4.5);
            }
        }
    }
}
