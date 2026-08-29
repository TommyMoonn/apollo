use egui::IconData;

const APPLICATION_ICON_PNG: &[u8] = include_bytes!("../../assets/apollo-logo.png");

/// Returns the repository-owned icon used by native Apollo windows.
pub fn application_icon() -> IconData {
    eframe::icon_data::from_png_bytes(APPLICATION_ICON_PNG)
        .expect("the embedded Apollo application icon must remain a valid PNG")
}

#[cfg(test)]
mod tests {
    use super::application_icon;

    #[test]
    fn embedded_application_icon_is_square_rgba_artwork() {
        let icon = application_icon();

        assert_eq!((icon.width, icon.height), (500, 500));
        assert_eq!(icon.rgba.len(), 500 * 500 * 4);
        assert_eq!(icon.rgba[3], 0);
        assert!(
            icon.rgba
                .chunks_exact(4)
                .any(|pixel| pixel == [0, 0, 0, 255])
        );
        assert!(
            icon.rgba
                .chunks_exact(4)
                .any(|pixel| pixel == [255, 255, 255, 255])
        );
    }
}
