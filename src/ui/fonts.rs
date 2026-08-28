use std::{collections::BTreeMap, sync::Arc};

use egui::{Context, FontData, FontDefinitions, FontFamily, FontId, TextStyle};

pub const PRIMARY_UI_FAMILY_NAME: &str = "IBM Plex Sans";
pub const DISPLAY_FAMILY_NAME: &str = "Fira Code";

const PRIMARY_FONT_DATA_NAME: &str = "apollo-ibm-plex-sans-regular";
const DISPLAY_FONT_DATA_NAME: &str = "apollo-fira-code-regular";
const PRIMARY_FONT_BYTES: &[u8] = include_bytes!("../../assets/fonts/IBMPlexSans-Regular.ttf");
const DISPLAY_FONT_BYTES: &[u8] = include_bytes!("../../assets/fonts/FiraCode-Regular.ttf");
const WORDMARK_TEXT_STYLE_NAME: &str = "apollo-wordmark";
const TECHNICAL_TEXT_STYLE_NAME: &str = "apollo-technical";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypographyScale {
    pub wordmark: f32,
    pub heading: f32,
    pub body: f32,
    pub secondary: f32,
}

pub const TYPOGRAPHY_SCALE: TypographyScale = TypographyScale {
    wordmark: 26.0,
    heading: 16.0,
    body: 14.0,
    secondary: 12.0,
};

/// Quiet Focus wordmark tracking: -0.075 em at the 26 px display size.
pub const WORDMARK_LETTER_SPACING: f32 = TYPOGRAPHY_SCALE.wordmark * -0.075;

#[derive(Debug)]
struct TypographyConfiguration {
    font_definitions: FontDefinitions,
    text_styles: BTreeMap<TextStyle, FontId>,
}

pub fn install(context: &Context) {
    let TypographyConfiguration {
        font_definitions,
        text_styles,
    } = embedded_configuration();

    context.set_fonts(font_definitions);
    context.all_styles_mut(|style| style.text_styles.clone_from(&text_styles));
}

pub fn primary_ui_family() -> FontFamily {
    FontFamily::Name(PRIMARY_UI_FAMILY_NAME.into())
}

pub fn display_family() -> FontFamily {
    FontFamily::Name(DISPLAY_FAMILY_NAME.into())
}

pub fn wordmark_text_style() -> TextStyle {
    TextStyle::Name(WORDMARK_TEXT_STYLE_NAME.into())
}

pub fn technical_text_style() -> TextStyle {
    TextStyle::Name(TECHNICAL_TEXT_STYLE_NAME.into())
}

fn embedded_configuration() -> TypographyConfiguration {
    let mut font_definitions = FontDefinitions::empty();
    font_definitions.font_data.insert(
        PRIMARY_FONT_DATA_NAME.to_owned(),
        Arc::new(FontData::from_static(PRIMARY_FONT_BYTES)),
    );
    font_definitions.font_data.insert(
        DISPLAY_FONT_DATA_NAME.to_owned(),
        Arc::new(FontData::from_static(DISPLAY_FONT_BYTES)),
    );

    let primary_chain = vec![
        PRIMARY_FONT_DATA_NAME.to_owned(),
        DISPLAY_FONT_DATA_NAME.to_owned(),
    ];
    let display_chain = vec![
        DISPLAY_FONT_DATA_NAME.to_owned(),
        PRIMARY_FONT_DATA_NAME.to_owned(),
    ];

    font_definitions
        .families
        .insert(primary_ui_family(), primary_chain.clone());
    font_definitions
        .families
        .insert(display_family(), display_chain.clone());
    font_definitions
        .families
        .insert(FontFamily::Proportional, primary_chain);
    font_definitions
        .families
        .insert(FontFamily::Monospace, display_chain);

    let mut text_styles = BTreeMap::new();
    text_styles.insert(
        TextStyle::Heading,
        FontId::new(TYPOGRAPHY_SCALE.heading, primary_ui_family()),
    );
    text_styles.insert(
        TextStyle::Body,
        FontId::new(TYPOGRAPHY_SCALE.body, primary_ui_family()),
    );
    text_styles.insert(
        TextStyle::Button,
        FontId::new(TYPOGRAPHY_SCALE.body, primary_ui_family()),
    );
    text_styles.insert(
        TextStyle::Small,
        FontId::new(TYPOGRAPHY_SCALE.secondary, primary_ui_family()),
    );
    text_styles.insert(
        TextStyle::Monospace,
        FontId::new(TYPOGRAPHY_SCALE.secondary, display_family()),
    );
    text_styles.insert(
        wordmark_text_style(),
        FontId::new(TYPOGRAPHY_SCALE.wordmark, display_family()),
    );
    text_styles.insert(
        technical_text_style(),
        FontId::new(TYPOGRAPHY_SCALE.secondary, display_family()),
    );

    TypographyConfiguration {
        font_definitions,
        text_styles,
    }
}

#[cfg(test)]
fn has_supported_sfnt_header(bytes: &[u8]) -> bool {
    if bytes.len() < 12 {
        return false;
    }

    matches!(
        &bytes[..4],
        [0x00, 0x01, 0x00, 0x00] | b"OTTO" | b"true" | b"typ1"
    )
}

#[cfg(test)]
mod tests {
    use egui::{FontFamily, TextStyle};

    use super::{
        DISPLAY_FAMILY_NAME, DISPLAY_FONT_BYTES, DISPLAY_FONT_DATA_NAME, PRIMARY_FONT_BYTES,
        PRIMARY_FONT_DATA_NAME, PRIMARY_UI_FAMILY_NAME, TYPOGRAPHY_SCALE, WORDMARK_LETTER_SPACING,
        display_family, embedded_configuration, has_supported_sfnt_header, primary_ui_family,
        technical_text_style, wordmark_text_style,
    };

    #[test]
    fn embedded_font_resources_are_supported_sfnt_assets() {
        assert!(has_supported_sfnt_header(PRIMARY_FONT_BYTES));
        assert!(has_supported_sfnt_header(DISPLAY_FONT_BYTES));
    }

    #[test]
    fn embedded_font_configuration_contains_both_intended_families() {
        let configuration = embedded_configuration();

        assert!(
            configuration
                .font_definitions
                .font_data
                .contains_key(PRIMARY_FONT_DATA_NAME)
        );
        assert!(
            configuration
                .font_definitions
                .font_data
                .contains_key(DISPLAY_FONT_DATA_NAME)
        );
        assert_eq!(
            configuration.font_definitions.families[&primary_ui_family()][0],
            PRIMARY_FONT_DATA_NAME
        );
        assert_eq!(
            configuration.font_definitions.families[&display_family()][0],
            DISPLAY_FONT_DATA_NAME
        );
        assert_eq!(PRIMARY_UI_FAMILY_NAME, "IBM Plex Sans");
        assert_eq!(DISPLAY_FAMILY_NAME, "Fira Code");
    }

    #[test]
    fn body_and_standard_controls_resolve_to_primary_ui_family() {
        let configuration = embedded_configuration();

        for text_style in [
            TextStyle::Heading,
            TextStyle::Body,
            TextStyle::Button,
            TextStyle::Small,
        ] {
            assert_eq!(
                configuration.text_styles[&text_style].family,
                primary_ui_family()
            );
        }
        assert_eq!(
            configuration.font_definitions.families[&FontFamily::Proportional][0],
            PRIMARY_FONT_DATA_NAME
        );
    }

    #[test]
    fn display_and_technical_styles_resolve_to_fira_code() {
        let configuration = embedded_configuration();

        for text_style in [
            TextStyle::Monospace,
            wordmark_text_style(),
            technical_text_style(),
        ] {
            assert_eq!(
                configuration.text_styles[&text_style].family,
                display_family()
            );
        }
        assert_eq!(
            configuration.font_definitions.families[&FontFamily::Monospace][0],
            DISPLAY_FONT_DATA_NAME
        );
    }

    #[test]
    fn quiet_focus_wordmark_tracking_is_centralized() {
        assert_eq!(WORDMARK_LETTER_SPACING, -1.95);
    }

    #[test]
    fn typography_scale_and_roles_are_owned_centrally() {
        let configuration = embedded_configuration();

        assert_eq!(TYPOGRAPHY_SCALE.wordmark, 26.0);
        assert_eq!(TYPOGRAPHY_SCALE.heading, 16.0);
        assert_eq!(TYPOGRAPHY_SCALE.body, 14.0);
        assert_eq!(TYPOGRAPHY_SCALE.secondary, 12.0);
        assert_eq!(
            configuration.text_styles[&wordmark_text_style()].size,
            TYPOGRAPHY_SCALE.wordmark
        );
        assert_eq!(
            configuration.text_styles[&TextStyle::Body].size,
            TYPOGRAPHY_SCALE.body
        );
        assert_eq!(
            configuration.text_styles[&technical_text_style()].size,
            TYPOGRAPHY_SCALE.secondary
        );
    }
}
