use anyhow::{bail, Result};
use gdk_pixbuf::Pixbuf;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

const DECODE_TIMEOUT: Duration = Duration::from_secs(10);

fn open_image_timeout(path: &Path) -> Result<image::DynamicImage> {
    let path = path.to_owned();
    let path_for_err = path.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(image::open(&path));
    });
    match rx.recv_timeout(DECODE_TIMEOUT) {
        Ok(result) => Ok(result?),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            bail!("Timed out after {}s decoding {:?}", DECODE_TIMEOUT.as_secs(), path_for_err)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            bail!("Decoder thread panicked for {:?}", path_for_err)
        }
    }
}

pub fn thumbnail(path: &Path, size: u32) -> Result<Pixbuf> {
    let img = open_image_timeout(path)?;
    let thumb = img.thumbnail(size, size);
    let rgba = thumb.to_rgba8();
    let (w, h) = rgba.dimensions();
    let data = rgba.into_raw();
    Ok(Pixbuf::from_bytes(
        &glib::Bytes::from(&data),
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        w as i32,
        h as i32,
        (w * 4) as i32,
    ))
}

pub fn format_size(size: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut s = size as f64;
    let mut i = 0;
    while s >= 1024.0 && i < units.len() - 1 {
        s /= 1024.0;
        i += 1;
    }
    format!("{:.1} {}", s, units[i])
}
