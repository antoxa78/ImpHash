/// Shared image-open helper used by both hasher and preview.
///
/// Tries `image::open` first (handles everything the `image` crate supports,
/// including AVIF via the `avif-native` feature).  Falls back to `libheif-rs`
/// for HEIC / HEIF files, which the `image` crate does not support.
use anyhow::{bail, Result};
use image::DynamicImage;
use std::path::Path;

pub fn open_image(path: &Path) -> Result<DynamicImage> {
    // Fast path: image crate handles jpg, png, webp, avif, tiff, bmp, gif,
    // hdr, exr, tga, ico, pnm, qoi, ff, dds …
    match image::open(path) {
        Ok(img) => return Ok(img),
        Err(image::ImageError::Unsupported(_)) => {
            // Fall through to format-specific decoders below.
        }
        Err(e) => return Err(e.into()),
    }

    // HEIC / HEIF — requires libheif (via libheif-rs).
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    if matches!(ext.as_deref(), Some("heic") | Some("heif")) {
        return decode_heif(path);
    }

    bail!("Unsupported image format: {:?}", path)
}

#[cfg(feature = "heif")]
fn decode_heif(path: &Path) -> Result<DynamicImage> {
    // In libheif-rs 2.x the decode method lives on LibHeif, not on ImageHandle.
    // Correct call chain: LibHeif::new() -> .decode(&handle, colorspace, None)
    use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};

    let ctx = HeifContext::read_from_file(&path.to_string_lossy())?;
    let handle = ctx.primary_image_handle()?;
    let has_alpha = handle.has_alpha_channel();

    let chroma = if has_alpha { RgbChroma::Rgba } else { RgbChroma::Rgb };

    let lib = LibHeif::new();
    let img = lib.decode(&handle, ColorSpace::Rgb(chroma), None)?;

    let plane = img.planes().interleaved.ok_or_else(|| {
        anyhow::anyhow!("HEIF image has no interleaved plane")
    })?;

    let width = plane.width;
    let height = plane.height;
    // plane.data is a &[u8] slice; copy it before img is dropped
    let data = plane.data.to_vec();

    let dynamic = if has_alpha {
        let buf = image::RgbaImage::from_raw(width, height, data)
            .ok_or_else(|| anyhow::anyhow!("HEIF RGBA buffer size mismatch"))?;
        DynamicImage::ImageRgba8(buf)
    } else {
        let buf = image::RgbImage::from_raw(width, height, data)
            .ok_or_else(|| anyhow::anyhow!("HEIF RGB buffer size mismatch"))?;
        DynamicImage::ImageRgb8(buf)
    };
    Ok(dynamic)
}

#[cfg(not(feature = "heif"))]
fn decode_heif(path: &Path) -> Result<DynamicImage> {
    bail!(
        "HEIC/HEIF support is not compiled in (missing 'heif' feature): {:?}",
        path
    )
}
