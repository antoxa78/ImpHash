use anyhow::Result;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const IMAGE_EXTENSIONS: &[&str] = &[
    // JPEG
    "jpg", "jpeg",
    // PNG
    "png",
    // WebP
    "webp",
    // AVIF (decoded via image crate avif-native feature — requires libdav1d)
    "avif",
    // TIFF
    "tiff", "tif",
    // BMP
    "bmp",
    // GIF
    "gif",
    // HDR (Radiance RGBE)
    "hdr",
    // OpenEXR
    "exr",
    // TGA (Truevision)
    "tga",
    // ICO (Windows icon)
    "ico",
    // PNM family
    "pbm", "pgm", "ppm", "pam",
    // QOI (Quite OK Image)
    "qoi",
    // Farbfeld
    "ff",
    // DDS (DirectDraw Surface)
    "dds",
    // HEIC / HEIF (decoded via libheif-rs, requires 'heif' feature)
    "heic", "heif",
];

fn find_images(path: &Path) -> Result<Vec<PathBuf>> {
    let mut images = Vec::new();

    for entry in WalkDir::new(path)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if IMAGE_EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
                    images.push(path.to_path_buf());
                }
            }
        }
    }

    Ok(images)
}

pub fn find_images_multi(dirs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut all = Vec::new();
    for dir in dirs {
        let imgs = find_images(dir)?;
        all.extend(imgs);
    }
    all.sort();
    all.dedup();
    Ok(all)
}
