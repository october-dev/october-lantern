//! Images you add to Ask Lantern (Swift `ImageAttachments`): pasted, dropped or picked, saved as
//! private files in Lantern's screenshots folder, the one place the engine hands an agent images
//! from. The agent gets their paths; October's answer gets the first one.
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// At most this many on one message.
pub const LIMIT: usize = 6;
/// The kinds agents read, kept as they are.
const KINDS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];
/// Bigger files are re-saved smaller (PNG and JPEG), or refused.
const KEPT_BYTES: u64 = 10_000_000;
/// The longest side of a picture Lantern saves itself.
const LONGEST_SIDE: u32 = 2400;

/// Where they're saved: the engine's screenshots folder.
pub fn dir() -> PathBuf {
    crate::settings::data_dir().join("screenshots")
}

pub fn is_image(path: &Path) -> bool {
    path.extension().is_some_and(|ext| KINDS.contains(&ext.to_string_lossy().to_lowercase().as_str()))
}

/// Copies an image file into `dir` (a very large PNG or JPEG is re-saved smaller).
pub fn save_file(path: &Path, dir: &Path) -> Result<PathBuf> {
    if !is_image(path) {
        bail!("Only PNG, JPEG, GIF and WebP images can be added.");
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let ext = if ext == "jpeg" { "jpg".to_string() } else { ext };
    let size = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?.len();
    // Only a picture that can be read: one that can't would be added, then silently left out of
    // the question.
    image::ImageReader::open(path).and_then(|r| r.with_guessed_format()).ok().and_then(|r| r.into_dimensions().ok()).with_context(
        || format!("{} isn't a picture Lantern can read.", path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()),
    )?;
    if size <= KEPT_BYTES {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        return write(dir, &ext, &bytes);
    }
    let image = image::open(path).context("That image is too large (over 10 MB) to add.")?;
    save_rgba(image.to_rgba8(), dir)
}

/// Saves a picture (from the clipboard) as PNG, scaled down when it's very large.
pub fn save_rgba(image: image::RgbaImage, dir: &Path) -> Result<PathBuf> {
    let longest = image.width().max(image.height());
    let image = if longest > LONGEST_SIDE {
        let scale = LONGEST_SIDE as f32 / longest as f32;
        image::imageops::resize(
            &image,
            ((image.width() as f32 * scale) as u32).max(1),
            ((image.height() as f32 * scale) as u32).max(1),
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        image
    };
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png)?;
    write(dir, "png", &png.into_inner())
}

/// The images on the clipboard, saved: image files (copied in Explorer or Files), else a picture
/// (a screenshot, or Copy Image in a browser). Text wins over a picture, since apps like Excel
/// and Word copy a picture of the text along with it. Empty when there's nothing to add.
pub fn from_clipboard(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut board = arboard::Clipboard::new().context("reading the clipboard")?;
    if let Ok(files) = board.get().file_list()
        && !files.is_empty()
    {
        return files.iter().filter(|f| is_image(f)).map(|f| save_file(f, dir)).collect();
    }
    if board.get_text().is_ok_and(|text| !text.is_empty()) {
        return Ok(Vec::new());
    }
    let Ok(picture) = board.get_image() else { return Ok(Vec::new()) };
    let image = image::RgbaImage::from_raw(picture.width as u32, picture.height as u32, picture.bytes.into_owned())
        .context("the clipboard's picture couldn't be read")?;
    Ok(vec![save_rgba(image, dir)?])
}

/// Pictures for one Ask Lantern question (Swift `ImageAttachments.jpegsForOctober`): at most six,
/// smaller when there are several (October takes 4 MB together). `None` when one can't be read: a
/// question never goes without a picture the person added.
pub fn jpegs_for_october(paths: &[String]) -> Option<Vec<Vec<u8>>> {
    let paths = &paths[..paths.len().min(6)];
    if paths.len() <= 1 {
        return paths.iter().map(|p| jpeg_for_october(Path::new(p))).collect();
    }
    let decoded: Vec<image::RgbImage> = paths.iter().map(|p| image::open(p).ok().map(|i| i.to_rgb8())).collect::<Option<_>>()?;
    for (side, quality) in [(1280u32, 75u8), (960, 65), (720, 55)] {
        let mut out = Vec::new();
        for picture in &decoded {
            let longest = picture.width().max(picture.height());
            let scaled = if longest > side {
                let scale = side as f32 / longest as f32;
                image::imageops::resize(
                    picture,
                    ((picture.width() as f32 * scale) as u32).max(1),
                    ((picture.height() as f32 * scale) as u32).max(1),
                    image::imageops::FilterType::Triangle,
                )
            } else {
                picture.clone()
            };
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality).encode_image(&scaled).ok()?;
            out.push(jpeg);
        }
        if out.iter().map(Vec::len).sum::<usize>() <= 3_900_000 {
            return Some(out);
        }
    }
    None
}

/// The image as JPEG for October's answer (one picture; see `shots::jpeg`).
pub fn jpeg_for_october(path: &Path) -> Option<Vec<u8>> {
    super::shots::jpeg(&std::fs::read(path).ok()?)
}

/// The lines that tell an agent where the images are (Swift `Assistant.imageLines`).
pub fn image_lines(images: &[String]) -> String {
    match images {
        [] => String::new(),
        [one] => format!("\n\nAn image I added (open it to see it): {one}"),
        many => {
            format!("\n\nImages I added (open them to see them):\n{}", many.iter().map(|i| format!("- {i}")).collect::<Vec<_>>().join("\n"))
        }
    }
}

fn write(dir: &Path, ext: &str, bytes: &[u8]) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let path = dir.join(format!("added-{stamp}-{}.{ext}", &uuid::Uuid::new_v4().simple().to_string()[..6]));
    crate::settings::private_write(&path, bytes)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lantern-images-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn image_files_are_copied_and_other_files_refused() {
        let dir = temp();
        let picture = |format: image::ImageFormat| {
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image::RgbaImage::new(3, 3)).write_to(&mut bytes, format).unwrap();
            bytes.into_inner()
        };
        let source = dir.join("Shot.JPEG");
        let jpeg = picture(image::ImageFormat::Jpeg);
        std::fs::write(&source, &jpeg).unwrap();
        let saved = save_file(&source, &dir.join("screenshots")).unwrap();
        assert!(saved.starts_with(dir.join("screenshots")) && saved.extension().unwrap() == "jpg", "{}", saved.display());
        assert_eq!(std::fs::read(&saved).unwrap(), jpeg);
        // GIF and WebP are read and sent to October like PNG and JPEG.
        for (name, format) in [("a.gif", image::ImageFormat::Gif), ("a.webp", image::ImageFormat::WebP)] {
            let path = dir.join(name);
            std::fs::write(&path, picture(format)).unwrap();
            let saved = save_file(&path, &dir.join("screenshots")).unwrap();
            assert!(jpeg_for_october(&saved).is_some(), "{name} is sent, not dropped");
        }
        // Several go together, smaller; one that can't be read stops them all.
        let paths: Vec<String> = ["a.gif", "a.webp", "Shot.JPEG"].iter().map(|n| dir.join(n).to_string_lossy().into_owned()).collect();
        assert_eq!(jpegs_for_october(&paths).map(|j| j.len()), Some(3));
        let missing = vec![paths[0].clone(), dir.join("gone.png").to_string_lossy().into_owned()];
        assert!(jpegs_for_october(&missing).is_none());
        // Something named like a picture that isn't one is refused when added.
        let fake = dir.join("fake.png");
        std::fs::write(&fake, b"not a picture").unwrap();
        assert!(save_file(&fake, &dir).is_err());
        let text = dir.join("notes.txt");
        std::fs::write(&text, b"hi").unwrap();
        assert!(save_file(&text, &dir).is_err());
        assert!(!is_image(Path::new("photo.heic")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_large_picture_is_saved_as_a_smaller_png() {
        let dir = temp();
        let saved = save_rgba(image::RgbaImage::from_pixel(4800, 1200, image::Rgba([10, 20, 30, 255])), &dir).unwrap();
        let back = image::open(&saved).unwrap();
        assert_eq!((back.width(), back.height()), (2400, 600));
        assert!(jpeg_for_october(&saved).is_some_and(|jpeg| jpeg.starts_with(&[0xff, 0xd8])));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn agents_are_told_where_the_images_are() {
        assert_eq!(image_lines(&[]), "");
        assert!(image_lines(&["/s/a.png".into()]).ends_with("/s/a.png"));
        assert!(image_lines(&["/s/a.png".into(), "/s/b.png".into()]).contains("- /s/a.png\n- /s/b.png"));
    }
}
