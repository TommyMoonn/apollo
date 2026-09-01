use std::{path::Path, sync::Arc};

use image::DynamicImage;

pub const THUMBNAIL_MAXIMUM_EDGE: u32 = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameCaptureThumbnail {
    size: [usize; 2],
    rgba: Arc<[u8]>,
}

impl FrameCaptureThumbnail {
    pub fn from_rgba(size: [usize; 2], rgba: Vec<u8>) -> Option<Self> {
        let expected_length = size[0].checked_mul(size[1])?.checked_mul(4)?;
        (size[0] > 0 && size[1] > 0 && rgba.len() == expected_length).then(|| Self {
            size,
            rgba: Arc::from(rgba),
        })
    }

    pub fn size(&self) -> [usize; 2] {
        self.size
    }

    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

pub fn decode_frame_capture_thumbnail(path: &Path) -> Result<FrameCaptureThumbnail, String> {
    let image = image::ImageReader::open(path)
        .map_err(|error| format!("captured image could not be opened: {error}"))?
        .with_guessed_format()
        .map_err(|error| format!("captured image format could not be identified: {error}"))?
        .decode()
        .map_err(|error| format!("captured image could not be decoded: {error}"))?;
    Ok(bounded_thumbnail(image))
}

fn bounded_thumbnail(image: DynamicImage) -> FrameCaptureThumbnail {
    let thumbnail =
        if image.width() <= THUMBNAIL_MAXIMUM_EDGE && image.height() <= THUMBNAIL_MAXIMUM_EDGE {
            image
        } else {
            image.thumbnail(THUMBNAIL_MAXIMUM_EDGE, THUMBNAIL_MAXIMUM_EDGE)
        };
    let rgba = thumbnail.to_rgba8();
    FrameCaptureThumbnail::from_rgba(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.into_raw(),
    )
    .expect("decoded RGBA image owns non-zero dimensions and complete pixel data")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use image::DynamicImage;

    use super::{THUMBNAIL_MAXIMUM_EDGE, bounded_thumbnail, decode_frame_capture_thumbnail};

    static NEXT_IMAGE: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn landscape_thumbnail_preserves_aspect_ratio_and_bounds_maximum_edge() {
        let preview = bounded_thumbnail(DynamicImage::new_rgba8(2_048, 1_024));

        assert_eq!(preview.size(), [512, 256]);
        assert_eq!(preview.rgba().len(), 512 * 256 * 4);
    }

    #[test]
    fn portrait_thumbnail_preserves_aspect_ratio_and_bounds_maximum_edge() {
        let preview = bounded_thumbnail(DynamicImage::new_rgba8(900, 1_800));

        assert_eq!(preview.size(), [256, 512]);
        assert!(preview.size().into_iter().max().unwrap() <= THUMBNAIL_MAXIMUM_EDGE as usize);
    }

    #[test]
    fn small_capture_is_not_upscaled_for_its_thumbnail() {
        let preview = bounded_thumbnail(DynamicImage::new_rgba8(160, 90));

        assert_eq!(preview.size(), [160, 90]);
    }

    #[test]
    fn rgba_constructor_rejects_invalid_dimensions_and_pixel_length() {
        assert!(super::FrameCaptureThumbnail::from_rgba([0, 1], Vec::new()).is_none());
        assert!(super::FrameCaptureThumbnail::from_rgba([2, 1], vec![0; 4]).is_none());
        assert!(super::FrameCaptureThumbnail::from_rgba([2, 1], vec![0; 8]).is_some());
    }

    #[test]
    fn published_png_is_decoded_into_bounded_rgba_preview_data() {
        let path = std::env::temp_dir().join(format!(
            "apollo-frame-thumbnail-{}-{}.png",
            std::process::id(),
            NEXT_IMAGE.fetch_add(1, Ordering::Relaxed)
        ));
        DynamicImage::new_rgba8(1_024, 512).save(&path).unwrap();

        let preview = decode_frame_capture_thumbnail(&path).unwrap();

        assert_eq!(preview.size(), [512, 256]);
        assert_eq!(preview.rgba().len(), 512 * 256 * 4);
        fs::remove_file(path).unwrap();
    }
}
