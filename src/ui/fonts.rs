use std::{
    collections::BTreeMap,
    error::Error,
    fmt, fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use egui::{Context, FontData, FontDefinitions, FontFamily, FontId, TextStyle};

pub const PRIMARY_UI_FAMILY_NAME: &str = "IBM Plex Sans";
pub const DISPLAY_FAMILY_NAME: &str = "Fira Code";

const PRIMARY_FONT_DATA_NAME: &str = "apollo-ibm-plex-sans-regular";
const DISPLAY_FONT_DATA_NAME: &str = "apollo-fira-code-regular";
const PRIMARY_FONT_FILE_NAME: &str = "IBMPlexSans-Regular.ttf";
const DISPLAY_FONT_FILE_NAME: &str = "FiraCode-Regular.ttf";
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

#[derive(Debug)]
pub enum FontSetupError {
    MissingAsset {
        family: &'static str,
        path: PathBuf,
    },
    ReadAsset {
        family: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    InvalidAsset {
        family: &'static str,
        path: PathBuf,
    },
}

impl fmt::Display for FontSetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingAsset { family, path } => write!(
                formatter,
                "required {family} font asset is missing: {}",
                path.display()
            ),
            Self::ReadAsset {
                family,
                path,
                source,
            } => write!(
                formatter,
                "could not read required {family} font asset {}: {source}",
                path.display()
            ),
            Self::InvalidAsset { family, path } => write!(
                formatter,
                "required {family} font asset is not a supported TrueType/OpenType font: {}",
                path.display()
            ),
        }
    }
}

impl Error for FontSetupError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ReadAsset { source, .. } => Some(source),
            Self::MissingAsset { .. } | Self::InvalidAsset { .. } => None,
        }
    }
}

#[derive(Debug)]
struct FontAssets {
    primary_ui: Vec<u8>,
    display: Vec<u8>,
}

#[derive(Debug)]
struct TypographyConfiguration {
    font_definitions: FontDefinitions,
    text_styles: BTreeMap<TextStyle, FontId>,
}

pub fn install(context: &Context) -> Result<(), FontSetupError> {
    let configuration = configuration_from_directory(repository_font_directory())?;
    let TypographyConfiguration {
        font_definitions,
        text_styles,
    } = configuration;

    context.set_fonts(font_definitions);
    context.all_styles_mut(|style| style.text_styles.clone_from(&text_styles));

    Ok(())
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

fn repository_font_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("fonts")
}

fn configuration_from_directory(
    font_directory: impl AsRef<Path>,
) -> Result<TypographyConfiguration, FontSetupError> {
    let assets = FontAssets::load(font_directory.as_ref())?;
    Ok(TypographyConfiguration::from_assets(assets))
}

impl FontAssets {
    fn load(font_directory: &Path) -> Result<Self, FontSetupError> {
        Ok(Self {
            primary_ui: load_font_asset(
                font_directory,
                PRIMARY_UI_FAMILY_NAME,
                PRIMARY_FONT_FILE_NAME,
            )?,
            display: load_font_asset(font_directory, DISPLAY_FAMILY_NAME, DISPLAY_FONT_FILE_NAME)?,
        })
    }
}

impl TypographyConfiguration {
    fn from_assets(assets: FontAssets) -> Self {
        let mut font_definitions = FontDefinitions::empty();
        font_definitions.font_data.insert(
            PRIMARY_FONT_DATA_NAME.to_owned(),
            Arc::new(FontData::from_owned(assets.primary_ui)),
        );
        font_definitions.font_data.insert(
            DISPLAY_FONT_DATA_NAME.to_owned(),
            Arc::new(FontData::from_owned(assets.display)),
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

        Self {
            font_definitions,
            text_styles,
        }
    }
}

fn load_font_asset(
    font_directory: &Path,
    family: &'static str,
    file_name: &str,
) -> Result<Vec<u8>, FontSetupError> {
    let path = font_directory.join(file_name);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Err(FontSetupError::MissingAsset { family, path });
        }
        Err(source) => {
            return Err(FontSetupError::ReadAsset {
                family,
                path,
                source,
            });
        }
    };

    if !has_supported_sfnt_header(&bytes) {
        return Err(FontSetupError::InvalidAsset { family, path });
    }

    Ok(bytes)
}

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
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use egui::{FontFamily, TextStyle};

    use super::{
        DISPLAY_FAMILY_NAME, DISPLAY_FONT_DATA_NAME, FontAssets, FontSetupError,
        PRIMARY_FONT_DATA_NAME, PRIMARY_UI_FAMILY_NAME, TYPOGRAPHY_SCALE, TypographyConfiguration,
        configuration_from_directory, display_family, primary_ui_family, repository_font_directory,
        technical_text_style, wordmark_text_style,
    };

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn font_definitions_contain_both_intended_families() {
        let configuration = test_configuration();

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
        let configuration = test_configuration();

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
        let configuration = test_configuration();

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
    fn missing_font_setup_fails_deterministically() {
        let directory = TestDirectory::new();

        let error = configuration_from_directory(directory.path()).unwrap_err();

        match error {
            FontSetupError::MissingAsset { family, path } => {
                assert_eq!(family, PRIMARY_UI_FAMILY_NAME);
                assert!(path.ends_with("IBMPlexSans-Regular.ttf"));
            }
            other => panic!("expected missing font asset error, got {other:?}"),
        }
    }

    #[test]
    fn repository_font_assets_build_configuration() {
        let font_directory = repository_font_directory();
        let configuration = configuration_from_directory(&font_directory).unwrap_or_else(|error| {
            panic!(
                "repository font assets must be present and loadable from {}: {error}",
                font_directory.display()
            )
        });

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
    }

    #[test]
    fn typography_scale_and_roles_are_owned_centrally() {
        let configuration = test_configuration();

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

    fn test_configuration() -> TypographyConfiguration {
        TypographyConfiguration::from_assets(FontAssets {
            primary_ui: test_sfnt_bytes(),
            display: test_sfnt_bytes(),
        })
    }

    fn test_sfnt_bytes() -> Vec<u8> {
        vec![0x00, 0x01, 0x00, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]
    }

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new() -> Self {
            let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "apollo-font-test-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("test font directory should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
