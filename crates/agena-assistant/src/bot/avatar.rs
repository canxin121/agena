//! Bounded, portable bot avatars shared by API clients and the registry.

use std::io::Cursor;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, imageops::FilterType};

use crate::{Error, Result};

pub const BOT_AVATAR_EDGE: u32 = 128;
pub const MAX_BOT_AVATAR_FILE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_BOT_AVATAR_DATA_URL_BYTES: usize = 256 * 1024;
pub const MAX_BOT_AVATAR_IMAGE_EDGE: u32 = 8192;
pub const MAX_BOT_AVATAR_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;
const PNG_PREFIX: &str = "data:image/png;base64,";

/// Validate an inline avatar and normalize it to a 128-pixel PNG thumbnail.
/// Call this on a blocking worker when handling an asynchronous request.
pub fn normalize_bot_avatar(data_url: &str) -> Result<String> {
    if data_url.len() > MAX_BOT_AVATAR_DATA_URL_BYTES {
        return Err(Error::Invalid(
            "bot avatar data URL must not exceed 256 KiB",
        ));
    }
    let (encoded, declared_format) = [
        (PNG_PREFIX, ImageFormat::Png),
        ("data:image/jpeg;base64,", ImageFormat::Jpeg),
        ("data:image/webp;base64,", ImageFormat::WebP),
    ]
    .into_iter()
    .find_map(|(prefix, format)| data_url.strip_prefix(prefix).map(|data| (data, format)))
    .ok_or(Error::Invalid(
        "bot avatar must be a base64 PNG, JPEG or WebP data URL",
    ))?;
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| Error::Invalid("bot avatar contains invalid base64"))?;
    thumbnail(&bytes, Some(declared_format))
}

/// Convert a local PNG, JPEG or WebP file into the portable avatar contract.
/// Reading the file must also be bounded by `MAX_BOT_AVATAR_FILE_BYTES`.
pub fn bot_avatar_from_bytes(bytes: &[u8]) -> Result<String> {
    if bytes.len() > MAX_BOT_AVATAR_FILE_BYTES {
        return Err(Error::Invalid("bot avatar file must not exceed 5 MiB"));
    }
    thumbnail(bytes, None)
}

fn thumbnail(bytes: &[u8], declared_format: Option<ImageFormat>) -> Result<String> {
    let invalid_image =
        || Error::Invalid("bot avatar must contain a valid PNG, JPEG or WebP image");
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| invalid_image())?;
    let format = reader.format().ok_or_else(invalid_image)?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err(invalid_image());
    }
    if declared_format.is_some_and(|declared| declared != format) {
        return Err(Error::Invalid(
            "bot avatar MIME type does not match its image",
        ));
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_BOT_AVATAR_IMAGE_EDGE);
    limits.max_image_height = Some(MAX_BOT_AVATAR_IMAGE_EDGE);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| invalid_image())?;
    let (width, height) = decoder.dimensions();
    if width == 0
        || height == 0
        || u64::from(width) * u64::from(height) > MAX_BOT_AVATAR_PIXELS
        || decoder.total_bytes() > MAX_DECODED_BYTES
    {
        return Err(Error::Invalid(
            "bot avatar exceeds the image dimensions or decoded size limit",
        ));
    }
    let orientation = decoder.orientation().map_err(|_| invalid_image())?;
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(|_| invalid_image())?;
    decoded.apply_orientation(orientation);
    // Crop before resizing. Resizing an extreme aspect ratio to fill a square
    // first would allocate a huge intermediate (8192×1 becomes 1048576×128).
    // Borrowing the square also avoids copying the full decoded image.
    let edge = decoded.width().min(decoded.height());
    let cropped = image::imageops::crop_imm(
        &decoded,
        (decoded.width() - edge) / 2,
        (decoded.height() - edge) / 2,
        edge,
        edge,
    );
    let thumbnail = DynamicImage::ImageRgba8(image::imageops::resize(
        &*cropped,
        BOT_AVATAR_EDGE,
        BOT_AVATAR_EDGE,
        FilterType::Lanczos3,
    ));
    let mut output = Cursor::new(Vec::new());
    thumbnail
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|_| Error::Invalid("bot avatar could not be encoded"))?;
    let data_url = format!("{PNG_PREFIX}{}", STANDARD.encode(output.into_inner()));
    if data_url.len() > MAX_BOT_AVATAR_DATA_URL_BYTES {
        return Err(Error::Invalid("bot avatar thumbnail exceeds 256 KiB"));
    }
    Ok(data_url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GenericImageView, ImageBuffer, ImageEncoder, Rgb, RgbImage};

    fn encoded(image: &DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        image.write_to(&mut output, format).unwrap();
        output.into_inner()
    }

    fn decode_avatar(data_url: &str) -> DynamicImage {
        let bytes = STANDARD
            .decode(data_url.strip_prefix(PNG_PREFIX).unwrap())
            .unwrap();
        image::load_from_memory_with_format(&bytes, ImageFormat::Png).unwrap()
    }

    #[test]
    fn supported_files_become_portable_png_thumbnails() {
        let source = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(180, 96, Rgb([32, 128, 224])));
        for (format, mime) in [
            (ImageFormat::Png, "png"),
            (ImageFormat::Jpeg, "jpeg"),
            (ImageFormat::WebP, "webp"),
        ] {
            let bytes = encoded(&source, format);
            let avatar = bot_avatar_from_bytes(&bytes).unwrap();
            assert!(avatar.len() <= MAX_BOT_AVATAR_DATA_URL_BYTES);
            assert_eq!(decode_avatar(&avatar).dimensions(), (128, 128));
            assert_eq!(normalize_bot_avatar(&avatar).unwrap(), avatar);
            let inline = format!("data:image/{mime};base64,{}", STANDARD.encode(bytes));
            assert_eq!(normalize_bot_avatar(&inline).unwrap(), avatar);
        }
    }

    #[test]
    fn rectangular_images_are_cropped_at_the_center() {
        let source = RgbImage::from_fn(384, 128, |x, _| {
            if (128..256).contains(&x) {
                Rgb([0, 255, 0])
            } else {
                Rgb([255, 0, 0])
            }
        });
        let avatar =
            bot_avatar_from_bytes(&encoded(&DynamicImage::ImageRgb8(source), ImageFormat::Png))
                .unwrap();
        let output = decode_avatar(&avatar).to_rgb8();
        assert!(output.pixels().all(|pixel| pixel.0 == [0, 255, 0]));
    }

    #[test]
    fn extreme_aspect_ratios_crop_without_an_enlarged_intermediate() {
        for (width, height) in [
            (MAX_BOT_AVATAR_IMAGE_EDGE, 1),
            (1, MAX_BOT_AVATAR_IMAGE_EDGE),
        ] {
            let source = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(
                width,
                height,
                Rgb([32, 128, 224]),
            ));
            let avatar = bot_avatar_from_bytes(&encoded(&source, ImageFormat::Png)).unwrap();
            let output = decode_avatar(&avatar).to_rgb8();
            assert_eq!(output.dimensions(), (128, 128));
            assert!(output.pixels().all(|pixel| pixel.0 == [32, 128, 224]));
        }
    }

    #[test]
    fn jpeg_orientation_is_applied_before_cropping() {
        let source = DynamicImage::ImageRgb8(RgbImage::from_fn(192, 128, |x, _| {
            if x < 96 {
                Rgb([255, 0, 0])
            } else {
                Rgb([0, 0, 255])
            }
        }));
        let jpeg = encoded(&source, ImageFormat::Jpeg);
        // EXIF orientation 6: rotate the stored image 90 degrees clockwise.
        let exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        let mut oriented = jpeg[..2].to_vec();
        oriented.extend_from_slice(&[0xff, 0xe1]);
        oriented.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        oriented.extend_from_slice(exif);
        oriented.extend_from_slice(&jpeg[2..]);
        let expected = image::load_from_memory_with_format(&jpeg, ImageFormat::Jpeg)
            .unwrap()
            .rotate90();
        assert_eq!(
            bot_avatar_from_bytes(&oriented).unwrap(),
            bot_avatar_from_bytes(&encoded(&expected, ImageFormat::Png)).unwrap(),
        );
    }

    #[test]
    fn invalid_images_and_mislabelled_data_urls_are_rejected() {
        let png = encoded(&DynamicImage::new_rgb8(1, 1), ImageFormat::Png);
        let wrong_mime = format!("data:image/jpeg;base64,{}", STANDARD.encode(&png));
        for input in [
            "https://example.com/avatar.png",
            "data:image/svg+xml;base64,PHN2Zy8+",
            "data:image/png;base64,?invalid",
            "data:image/png;base64,",
            &wrong_mime,
        ] {
            assert!(normalize_bot_avatar(input).is_err(), "accepted {input}");
        }
        assert!(bot_avatar_from_bytes(&png[..16]).is_err());
        let bmp = encoded(&DynamicImage::new_rgb8(1, 1), ImageFormat::Bmp);
        assert!(bot_avatar_from_bytes(&bmp).is_err());
    }

    #[test]
    fn file_and_inline_sizes_are_bounded_before_decoding() {
        let file = vec![0; MAX_BOT_AVATAR_FILE_BYTES + 1];
        assert!(matches!(
            bot_avatar_from_bytes(&file),
            Err(Error::Invalid("bot avatar file must not exceed 5 MiB"))
        ));
        let inline = "x".repeat(MAX_BOT_AVATAR_DATA_URL_BYTES + 1);
        assert!(matches!(
            normalize_bot_avatar(&inline),
            Err(Error::Invalid(
                "bot avatar data URL must not exceed 256 KiB"
            ))
        ));
    }

    #[test]
    fn compressed_images_cannot_bypass_pixel_limits() {
        for (width, height) in [(8193, 1), (4097, 4096)] {
            let pixels = vec![0; width as usize * height as usize];
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new_with_quality(
                &mut png,
                image::codecs::png::CompressionType::Fast,
                image::codecs::png::FilterType::NoFilter,
            )
            .write_image(&pixels, width, height, image::ExtendedColorType::L8)
            .unwrap();
            assert!(png.len() < MAX_BOT_AVATAR_FILE_BYTES);
            assert!(bot_avatar_from_bytes(&png).is_err());
        }
    }
}
