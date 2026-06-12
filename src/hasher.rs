use anyhow::{bail, Result};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(1);

fn open_image_timeout(path: &Path, cancel: &AtomicBool) -> Result<image::DynamicImage> {
    let path_buf = path.to_owned();
    let path_for_err = path_buf.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(image::open(&path_buf));
    });
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("Cancelled by user");
        }
        match rx.recv_timeout(POLL_INTERVAL) {
            Ok(result) => return Ok(result?),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bail!("Decoder thread panicked for {:?}", path_for_err)
            }
        }
    }
}

// NOTE: stretch_contrast was removed — it was the root cause of false
// positives on low-contrast images.  dHash is inherently contrast-invariant
// (only relative pixel comparisons matter), so normalisation is unnecessary
// and amplifies noise in dark / low-dynamic-range images.

/// Minimum standard deviation (on the 9×8 resized grid) below which the
/// image is considered too flat to produce a meaningful dHash.
/// Empirically, natural photos land above 15–20; synthetic gradients and
/// dark/blank frames fall below 5.
const LOW_VARIANCE_THRESHOLD: f64 = 5.0;

/// Compute dHash from a grayscale image.
/// Returns `(hash, is_low_variance)`.  When `is_low_variance` is true the
/// hash is unreliable and the caller should either skip the image or
/// require an exact (distance == 0) match.
fn dhash_from_gray(gray: &image::GrayImage) -> (u64, bool) {
    // Resize first — no contrast stretching.
    // dHash is inherently contrast-invariant (only relative comparisons),
    // so stretch_contrast is unnecessary and actively harmful for
    // low-contrast images (amplifies noise → false matches).
    let resized = image::imageops::resize(
        gray, 9, 8,
        image::imageops::FilterType::Triangle,
    );

    // Variance guard: if the resized tile is nearly flat, the hash will be
    // dominated by rounding noise and many unrelated images will collide.
    let pixels: Vec<f64> = resized.pixels().map(|p| p[0] as f64).collect();
    let n = pixels.len() as f64;
    let mean = pixels.iter().sum::<f64>() / n;
    let variance = pixels.iter().map(|&p| (p - mean) * (p - mean)).sum::<f64>() / n;
    let stddev = variance.sqrt();
    let low_var = stddev < LOW_VARIANCE_THRESHOLD;

    let mut hash: u64 = 0;
    let mut bit = 0;
    for y in 0..8 {
        for x in 0..8 {
            let left = resized.get_pixel(x, y)[0];
            let right = resized.get_pixel(x + 1, y)[0];
            if left > right {
                hash |= 1 << bit;
            }
            bit += 1;
        }
    }
    (hash, low_var)
}

pub fn dhash(path: &Path, cancel: &AtomicBool) -> Result<(u64, bool)> {
    let img = open_image_timeout(path, cancel)?;
    Ok(dhash_from_gray(&img.to_luma8()))
}

/// Compute 4 dHashes: original, 90° rotated, 180° rotated, 270° rotated.
/// h0 uses the full image (same as regular `dhash`).
/// h90/h180/h270 rotate a 256×256 intermediate to avoid aliasing at 64×64.
/// Returns `([h0, h90, h180, h270], low_variance)`.
pub fn dhash_rotations(path: &Path, cancel: &AtomicBool) -> Result<([u64; 4], bool)> {
    let img = open_image_timeout(path, cancel)?;
    let gray = img.to_luma8();

    let (h0, low_var) = dhash_from_gray(&gray);

    let medium = image::imageops::resize(
        &gray, 256, 256,
        image::imageops::FilterType::Triangle,
    );

    let (h90, _) = dhash_from_gray(&image::imageops::rotate90(&medium));
    let (h180, _) = dhash_from_gray(&image::imageops::rotate180(&medium));
    let (h270, _) = dhash_from_gray(&image::imageops::rotate270(&medium));

    Ok(([h0, h90, h180, h270], low_var))
}

/// A hash is "degenerate" when all 8 bytes are nearly identical — this
/// happens when a dark/flat image is rotated and resized to 9×8, producing
/// a near-uniform grid.  These hashes cluster tightly in hash-space and
/// cause massive false-positive rates in rotation matching.
pub fn is_degenerate_hash(h: u64) -> bool {
    let bytes = h.to_le_bytes();
    let first = bytes[0];
    bytes.iter().all(|&b| (b ^ first).count_ones() <= 1)
}

/// Minimum Hamming distance for rotation-invariant comparison.
///
/// Checks whether image B is a rotation of image A (a[0] vs all of b)
/// and vice-versa (b[0] vs all of a).  This is 8 comparisons instead
/// of the old 16 (all-vs-all), which dramatically reduces false
/// positives on dark/low-contrast images where rotation hashes are
/// noise-dominated.
///
/// Degenerate hashes (all bytes within 1 bit of each other, e.g.
/// f0f0f0f0f0f0f0f0) are skipped entirely — they're noise attractors
/// that cause unrelated dark images to match.
pub fn rotation_min_distance(a: &[u64; 4], b: &[u64; 4]) -> u32 {
    let mut min_d = u32::MAX;
    // Is B a rotation of A?  (compare A's original hash against B's rotations)
    if !is_degenerate_hash(a[0]) {
        for hb in b.iter() {
            if is_degenerate_hash(*hb) { continue; }
            let d = (a[0] ^ hb).count_ones();
            if d < min_d { min_d = d; }
        }
    }
    // Is A a rotation of B?  (compare B's original hash against A's rotations)
    if !is_degenerate_hash(b[0]) {
        for ha in a.iter() {
            if is_degenerate_hash(*ha) { continue; }
            let d = (ha ^ b[0]).count_ones();
            if d < min_d { min_d = d; }
        }
    }
    min_d
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    static CANCEL_FALSE: AtomicBool = AtomicBool::new(false);

    #[test]
    fn test_dhash_duplicate_images() {
        let (h1, _) = dhash(Path::new("/tmp/imphash_test/photo1.png"), &CANCEL_FALSE).unwrap();
        let (h2, _) = dhash(Path::new("/tmp/imphash_test/photo1_copy.png"), &CANCEL_FALSE).unwrap();
        assert_eq!(h1, h2, "identical images should have same hash");
    }

    #[test]
    fn test_dhash_different_images() {
        let (h1, _) = dhash(Path::new("/tmp/imphash_test/photo1.png"), &CANCEL_FALSE).unwrap();
        let (h2, _) = dhash(Path::new("/tmp/imphash_test/photo2.png"), &CANCEL_FALSE).unwrap();
        assert_ne!(h1, h2, "different images should have different hashes");
    }

    #[test]
    fn test_dhash_compression_levels() {
        let (h_high, _) = dhash(Path::new("/tmp/imphash_test/test_highq.jpg"), &CANCEL_FALSE).unwrap();
        let (h_low, _) = dhash(Path::new("/tmp/imphash_test/test_lowq.jpg"), &CANCEL_FALSE).unwrap();
        let (h_png, _) = dhash(Path::new("/tmp/imphash_test/test_original.png"), &CANCEL_FALSE).unwrap();

        let high_low_dist = (h_high ^ h_low).count_ones();
        let high_png_dist = (h_high ^ h_png).count_ones();
        let low_png_dist = (h_low ^ h_png).count_ones();

        eprintln!("dHash high-q JPEG:  {:016x}", h_high);
        eprintln!("dHash low-q JPEG:   {:016x}", h_low);
        eprintln!("dHash PNG original: {:016x}", h_png);
        eprintln!("Hamming high vs low:  {}", high_low_dist);
        eprintln!("Hamming high vs PNG:  {}", high_png_dist);
        eprintln!("Hamming low vs PNG:   {}", low_png_dist);

        // These are the SAME image at different compressions.
        // Even between extreme quality levels, dHash should be close.
        assert!(
            high_low_dist <= 16,
            "high vs low JPEG distance {} > 16 — dHash not robust enough",
            high_low_dist
        );
        assert!(
            high_png_dist <= 16,
            "high JPEG vs PNG distance {} > 16",
            high_png_dist
        );
        assert!(
            low_png_dist <= 16,
            "low JPEG vs PNG distance {} > 16",
            low_png_dist
        );
    }

    #[test]
    fn test_dhash_natural_images_compression() {
        let (h_high, _) = dhash(Path::new("/tmp/imphash_test2/natural_highq.jpg"), &CANCEL_FALSE).unwrap();
        let (h_low, _) = dhash(Path::new("/tmp/imphash_test2/natural_lowq.jpg"), &CANCEL_FALSE).unwrap();
        let (h_med, _) = dhash(Path::new("/tmp/imphash_test2/natural_medium.jpg"), &CANCEL_FALSE).unwrap();
        let (h_png, _) = dhash(Path::new("/tmp/imphash_test2/natural_png.png"), &CANCEL_FALSE).unwrap();

        let high_low = (h_high ^ h_low).count_ones();
        let high_med = (h_high ^ h_med).count_ones();
        let high_png = (h_high ^ h_png).count_ones();
        let low_png = (h_low ^ h_png).count_ones();
        let med_low = (h_med ^ h_low).count_ones();

        eprintln!("=== Natural image dHash comparison ===");
        eprintln!("high-q JPEG:  {:016x}", h_high);
        eprintln!("medium JPEG:  {:016x}", h_med);
        eprintln!("low-q JPEG:   {:016x}", h_low);
        eprintln!("PNG:          {:016x}", h_png);
        eprintln!("high vs low:   {}", high_low);
        eprintln!("high vs med:   {}", high_med);
        eprintln!("high vs PNG:   {}", high_png);
        eprintln!("low vs PNG:    {}", low_png);
        eprintln!("med vs low:    {}", med_low);

        assert!(high_low <= 16, "high vs low distance {} > 16", high_low);
        assert!(high_med <= 16, "high vs med distance {} > 16", high_med);
        assert!(high_png <= 16, "high vs PNG distance {} > 16", high_png);
        assert!(low_png <= 16, "low vs PNG distance {} > 16", low_png);
        assert!(med_low <= 16, "med vs low distance {} > 16", med_low);
    }

    #[test]
    fn test_dhash_alpha_channel() {
        let (h_rgba, _) = dhash(Path::new("/tmp/imphash_test3/rgba.png"), &CANCEL_FALSE).unwrap();
        let (h_rgb_jpg, _) = dhash(Path::new("/tmp/imphash_test3/rgb.jpg"), &CANCEL_FALSE).unwrap();
        let (h_rgb_png, _) = dhash(Path::new("/tmp/imphash_test3/rgb.png"), &CANCEL_FALSE).unwrap();

        let rgba_rgbjpg = (h_rgba ^ h_rgb_jpg).count_ones();
        let rgba_rgbpng = (h_rgba ^ h_rgb_png).count_ones();

        eprintln!("=== Alpha channel test ===");
        eprintln!("RGBA PNG: {:016x}", h_rgba);
        eprintln!("RGB JPG:  {:016x}", h_rgb_jpg);
        eprintln!("RGB PNG:  {:016x}", h_rgb_png);
        eprintln!("RGBA vs RGB JPG: {}", rgba_rgbjpg);
        eprintln!("RGBA vs RGB PNG: {}", rgba_rgbpng);

        // Images with/without alpha channels should be near-identical
        assert!(rgba_rgbjpg <= 8, "RGBA vs RGB JPG distance {} > 8", rgba_rgbjpg);
        assert!(rgba_rgbpng <= 8, "RGBA vs RGB PNG distance {} > 8", rgba_rgbpng);
    }

    #[test]
    fn test_dhash_different_dimensions() {
        let (h_full, _) = dhash(Path::new("/tmp/imphash_test3/halfsize.jpg"), &CANCEL_FALSE).unwrap();
        let (h_half, _) = dhash(Path::new("/tmp/imphash_test3/halfsize_small.jpg"), &CANCEL_FALSE).unwrap();

        let dist = (h_full ^ h_half).count_ones();
        eprintln!("=== Different dimensions test ===");
        eprintln!("full:    {:016x}", h_full);
        eprintln!("half:    {:016x}", h_half);
        eprintln!("distance: {}", dist);

        assert!(dist <= 8, "different dimensions distance {} > 8", dist);
    }

    #[test]
    fn test_dhash_large_high_quality() {
        let (h_tiff_uncomp, _) = dhash(Path::new("/tmp/imphash_test4/large_uncompressed.tiff"), &CANCEL_FALSE).unwrap();
        let (h_tiff_comp, _) = dhash(Path::new("/tmp/imphash_test4/large_compressed.tiff"), &CANCEL_FALSE).unwrap();
        let (h_jpg_high, _) = dhash(Path::new("/tmp/imphash_test4/large_highq.jpg"), &CANCEL_FALSE).unwrap();
        let (h_jpg_low, _) = dhash(Path::new("/tmp/imphash_test4/large_lowq.jpg"), &CANCEL_FALSE).unwrap();
        let (h_png, _) = dhash(Path::new("/tmp/imphash_test4/large_png.png"), &CANCEL_FALSE).unwrap();

        eprintln!("=== Large high-quality image test ===");
        for (name, hash) in [("TIFF uncomp", h_tiff_uncomp), ("TIFF comp", h_tiff_comp),
                               ("JPEG high", h_jpg_high), ("JPEG low", h_jpg_low),
                               ("PNG", h_png)] {
            eprintln!("{:12}: {:016x}", name, hash);
        }

        let pairs = [
            ("TIFF uncomp vs comp", h_tiff_uncomp, h_tiff_comp),
            ("TIFF uncomp vs JPEG high", h_tiff_uncomp, h_jpg_high),
            ("TIFF uncomp vs JPEG low", h_tiff_uncomp, h_jpg_low),
            ("TIFF uncomp vs PNG", h_tiff_uncomp, h_png),
            ("TIFF comp vs JPEG high", h_tiff_comp, h_jpg_high),
            ("TIFF comp vs JPEG low", h_tiff_comp, h_jpg_low),
            ("JPEG high vs JPEG low", h_jpg_high, h_jpg_low),
            ("JPEG high vs PNG", h_jpg_high, h_png),
            ("JPEG low vs PNG", h_jpg_low, h_png),
        ];

        let mut max_dist = 0u32;
        for (label, a, b) in &pairs {
            let dist = (*a ^ *b).count_ones();
            eprintln!("{:25}: {}", label, dist);
            max_dist = max_dist.max(dist);
        }
        eprintln!("Max Hamming distance: {}", max_dist);

        // All are the SAME image; dHash should be close for all format variants
        assert!(
            max_dist <= 16,
            "max distance {} > 16 across all format variants",
            max_dist
        );
    }

    #[test]
    fn test_dhash_color_gamut() {
        let (h_srgb, _) = dhash(Path::new("/tmp/imphash_test3/srgb.jpg"), &CANCEL_FALSE).unwrap();
        let (h_adobergb, _) = dhash(Path::new("/tmp/imphash_test3/adobergb.jpg"), &CANCEL_FALSE).unwrap();

        let dist = (h_srgb ^ h_adobergb).count_ones();
        eprintln!("=== Color gamut test ===");
        eprintln!("sRGB:     {:016x}", h_srgb);
        eprintln!("AdobeRGB: {:016x}", h_adobergb);
        eprintln!("distance: {}", dist);

        // Simulated wider gamut may differ more
        assert!(dist <= 16, "sRGB vs AdobeRGB distance {} > 16", dist);
    }
}
