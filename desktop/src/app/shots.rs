//! Point & Ask's screenshots (Swift `ScreenshotStore` and `PointAsk.jpeg`).
//!
//! A screenshot sent to an agent is saved inside its project, in `.lantern/shots/`, only after Git
//! confirms that folder is ignored; Lantern adds `.lantern/` to the repository's local exclude list
//! (`.git/info/exclude`), never `.gitignore`. Anything else (no project, not a repository, a
//! protected one) goes to Lantern's private screenshots folder.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Saves `png` for an agent working in `cwd`; returns where it went.
pub fn save(png: &[u8], cwd: Option<&Path>, fallback: &Path) -> Result<PathBuf> {
    let name = format!("point-{}.png", uuid::Uuid::new_v4().hyphenated());
    if let Some(root) = cwd.and_then(|cwd| cwd.canonicalize().ok()) {
        let proposed = root.join(".lantern").join("shots");
        let inside = !proposed.exists() || proposed.canonicalize().is_ok_and(|p| p.starts_with(&root));
        if inside
            && exclude_from_git(&root)
            && git(&["check-ignore", "--quiet", "--", &format!(".lantern/shots/{name}")], &root).is_some_and(|(ok, _)| ok)
            && let Ok(path) = write_private(&proposed, &name, png)
        {
            return Ok(path);
        }
    }
    write_private(fallback, &name, png)
}

fn write_private(directory: &Path, name: &str, png: &[u8]) -> Result<PathBuf> {
    std::fs::create_dir_all(directory).with_context(|| format!("creating {}", directory.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700));
    }
    let path = directory.join(name);
    crate::settings::private_write(&path, png)?;
    Ok(path)
}

/// Makes sure `.lantern/` is ignored in the repository at `cwd` (local exclude list only).
pub fn exclude_from_git(cwd: &Path) -> bool {
    let probe = || {
        git(&["check-ignore", "--quiet", "--", &format!(".lantern/shots/{}", uuid::Uuid::new_v4().simple())], cwd).is_some_and(|(ok, _)| ok)
    };
    if probe() {
        return true;
    }
    let Some((true, exclude)) = git(&["rev-parse", "--path-format=absolute", "--git-path", "info/exclude"], cwd) else { return false };
    let exclude = PathBuf::from(exclude);
    if !exclude.is_absolute() {
        return false;
    }
    let current = std::fs::read_to_string(&exclude).unwrap_or_default();
    if !current.lines().any(|line| line == ".lantern/") {
        let separator = if current.is_empty() || current.ends_with('\n') { "" } else { "\n" };
        let added = format!("{current}{separator}# October Lantern screenshots\n.lantern/\n");
        if exclude.parent().is_some_and(|dir| std::fs::create_dir_all(dir).is_err()) || std::fs::write(&exclude, added).is_err() {
            return false;
        }
    }
    probe()
}

/// Runs git without hooks; `(succeeded, stdout)` or `None` when it couldn't run in 2 seconds.
fn git(arguments: &[&str], cwd: &Path) -> Option<(bool, String)> {
    let mut child = crate::os::command("git")
        .args(["-c", "core.hooksPath=/dev/null", "-C"])
        .arg(cwd)
        .args(arguments)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                use std::io::Read;
                child.stdout.take()?.read_to_string(&mut out).ok()?;
                return Some((status.success(), out.trim().to_string()));
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                return None;
            }
        }
    }
}

/// The screenshot as a JPEG for October's AI: at most 1600 px on its longer side and under
/// 2.5 MB (the server takes 4.5 MB and base64 adds a third), smaller if it has to be.
pub fn jpeg(png: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(png).ok()?.to_rgb8();
    for (side, quality) in [(1600u32, 82u8), (1600, 60), (1200, 60), (900, 50)] {
        let longest = image.width().max(image.height());
        let scaled = if longest > side {
            let scale = side as f32 / longest as f32;
            image::imageops::resize(
                &image,
                ((image.width() as f32 * scale) as u32).max(1),
                ((image.height() as f32 * scale) as u32).max(1),
                image::imageops::FilterType::Lanczos3,
            )
        } else {
            image.clone()
        };
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).encode_image(&scaled).ok()?;
        if out.len() <= 2_500_000 {
            return Some(out);
        }
    }
    None
}

/// Crops a full-screen PNG to `rect` (physical pixels, relative to the image) and re-encodes it.
pub fn crop(png: &[u8], x: u32, y: u32, width: u32, height: u32) -> Result<Vec<u8>> {
    crop_image(&image::load_from_memory(png)?.to_rgba8(), x, y, width, height)
}

/// Crops a captured display to `rect` (physical pixels, relative to the image) as PNG.
pub fn crop_image(image: &image::RgbaImage, x: u32, y: u32, width: u32, height: u32) -> Result<Vec<u8>> {
    anyhow::ensure!(width > 0 && height > 0, "nothing selected");
    let (x, y) = (x.min(image.width() - 1), y.min(image.height() - 1));
    let (width, height) = (width.min(image.width() - x), height.min(image.height() - y));
    let cropped = image::imageops::crop_imm(image, x, y, width, height).to_image();
    let mut out = std::io::Cursor::new(Vec::new());
    cropped.write_to(&mut out, image::ImageFormat::Png)?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_fn(width, height, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lantern-shots-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_repository_gets_the_shot_in_lantern_shots_excluded_locally() {
        let repo = temp("repo");
        if git(&["init", "-q"], &repo).is_none_or(|(ok, _)| !ok) {
            return; // no git here
        }
        let fallback = temp("fallback");
        let path = save(&png(4, 4), Some(&repo), &fallback).unwrap();
        assert!(path.starts_with(repo.canonicalize().unwrap().join(".lantern/shots")), "{}", path.display());
        let exclude = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
        assert!(exclude.contains("\n.lantern/\n") || exclude.starts_with("# October Lantern screenshots\n.lantern/\n"));
        assert!(!repo.join(".gitignore").exists(), "never .gitignore");
        // A second save doesn't add the line twice.
        save(&png(4, 4), Some(&repo), &fallback).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap().matches(".lantern/").count(), 1);
        let _ = std::fs::remove_dir_all(repo);
        let _ = std::fs::remove_dir_all(fallback);
    }

    #[test]
    fn outside_a_repository_it_goes_to_the_private_folder() {
        let plain = temp("plain");
        let fallback = temp("fallback");
        let path = save(&png(4, 4), Some(&plain), &fallback).unwrap();
        assert!(path.starts_with(&fallback));
        assert!(!plain.join(".lantern").exists());
        let _ = std::fs::remove_dir_all(plain);
        let _ = std::fs::remove_dir_all(fallback);
    }

    #[test]
    fn jpeg_caps_the_longer_side() {
        let jpeg = jpeg(&png(3200, 800)).unwrap();
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1600, 400));
        assert!(jpeg.len() <= 2_500_000);
    }

    #[test]
    fn crop_takes_the_selection_and_stays_inside_the_image() {
        let cropped = crop(&png(100, 50), 90, 40, 30, 30).unwrap();
        let image = image::load_from_memory(&cropped).unwrap();
        assert_eq!((image.width(), image.height()), (10, 10));
        assert!(crop(&png(10, 10), 0, 0, 0, 5).is_err());
    }
}
