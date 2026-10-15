//! Local dictation's speech runtime: whisper.cpp's `whisper-cli` and its multilingual base model.
//! Windows releases ship `whisper-cli.exe` next to Lantern; on Linux it's installed separately
//! (scripts/setup-dictation.sh, or a package). The model is downloaded on first use into
//! Lantern's data folder (`speech`), checked against its pinned SHA-256 before it's used, and
//! the settings point at it, as the setup scripts do.
use crate::settings::{Settings, data_dir};
use anyhow::{Context, Result};
use sha2::Digest;
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin";
pub const MODEL_SHA256: &str = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe";
pub const MODEL_NAME: &str = "ggml-base.bin";
/// Its size, for progress when the server doesn't say.
const MODEL_BYTES: u64 = 147_951_465;
/// Anything much larger isn't the model.
const MODEL_LIMIT: u64 = 256 * 1024 * 1024;
const PROGRAM: &str = "whisper-cli";

/// What pressing the mic finds.
#[derive(Debug, PartialEq)]
pub enum Readiness {
    Ready,
    /// The runtime is here but the model isn't yet: download it, then listen.
    NeedsModel {
        whisper: PathBuf,
    },
    /// No `whisper-cli` to run.
    NoRuntime,
}

/// One step of dictation's first-use setup, sent from the download to the app.
pub enum Setup {
    Progress(u8),
    Done(Result<(), String>),
}

pub fn speech_dir() -> PathBuf {
    data_dir().join("speech")
}

pub fn readiness(settings: &Settings) -> Readiness {
    match (whisper(settings), model(settings)) {
        (None, _) => Readiness::NoRuntime,
        (Some(whisper), None) => Readiness::NeedsModel { whisper },
        (Some(_), Some(_)) => Readiness::Ready,
    }
}

/// What to say when there's no `whisper-cli` at all.
pub fn missing_runtime(windows: bool) -> &'static str {
    if windows {
        "Dictation's speech runtime (whisper-cli.exe) is missing from Lantern's folder. Reinstall Lantern and try again."
    } else {
        "Dictation needs whisper.cpp's whisper-cli. Install it with your package manager, or run scripts/setup-dictation.sh from Lantern's source."
    }
}

/// The toast while the model downloads.
pub fn progress(percent: u8) -> String {
    format!("Setting up dictation (148 MB, once)… {percent}%")
}

/// The toast when setting up failed.
pub fn failed(error: &str) -> String {
    format!("Couldn't set up dictation: {}. Try again.", error.trim().trim_end_matches('.'))
}

/// The settings with the runtime and model they resolve to, for transcribing.
pub fn resolved(mut settings: Settings) -> Settings {
    if let Some(whisper) = whisper(&settings) {
        settings.whisper = whisper.to_string_lossy().into_owned();
    }
    if let Some(model) = model(&settings) {
        settings.whisper_model = model.to_string_lossy().into_owned();
    }
    settings
}

/// The `whisper-cli` to run: the one in the settings when it's a path that exists, otherwise
/// the one next to Lantern's own program (a Windows release ships it there), otherwise PATH's.
pub fn whisper(settings: &Settings) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok();
    let path = std::env::var_os("PATH");
    find_whisper(&settings.whisper, exe.as_deref().and_then(Path::parent), path.as_deref())
}

fn find_whisper(configured: &str, beside: Option<&Path>, path: Option<&OsStr>) -> Option<PathBuf> {
    let configured = configured.trim();
    let given = Path::new(configured);
    let name = if configured.is_empty() || given.is_absolute() || given.components().count() > 1 {
        if !configured.is_empty() && given.is_file() {
            return Some(given.to_path_buf());
        }
        // A runtime the setup script installed and that has since gone: use the bundled one.
        PROGRAM
    } else {
        configured
    };
    let names = [format!("{name}{}", std::env::consts::EXE_SUFFIX), name.to_string()];
    beside
        .map(Path::to_path_buf)
        .into_iter()
        .chain(path.map(|path| std::env::split_paths(path).collect::<Vec<_>>()).unwrap_or_default())
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

/// Whether this `whisper-cli` is the one shipped next to Lantern (its settings then keep the bare
/// name, so they follow Lantern to an updated version's folder).
fn bundled(whisper: &Path) -> bool {
    let beside = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    beside.is_some_and(|dir| whisper.parent() == Some(dir.as_path()))
}

/// The model in the settings, or the one a first-use setup downloaded.
pub fn model(settings: &Settings) -> Option<PathBuf> {
    let configured = Path::new(&settings.whisper_model);
    if !settings.whisper_model.is_empty() && configured.is_file() {
        return Some(configured.to_path_buf());
    }
    let ours = speech_dir().join(MODEL_NAME);
    ours.is_file().then_some(ours)
}

/// Downloads the model, then points the settings at it and at `whisper`. Runs off the UI thread.
pub fn set_up(whisper: &Path, mut progress: impl FnMut(u8)) -> Result<()> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        // A large file on a slow connection; only a stalled one ends this.
        .timeout_recv_body(Some(Duration::from_secs(3 * 3600)))
        .build()
        .into();
    let mut response = agent.get(MODEL_URL).header("User-Agent", "October-Lantern").call().map_err(|error| match error {
        ureq::Error::StatusCode(code) => anyhow::anyhow!("the speech model's server answered {code}"),
        other => anyhow::anyhow!("couldn't reach the speech model's server ({other})"),
    })?;
    let body = response.body_mut();
    let total = body.content_length();
    let model = install_model(body.with_config().limit(MODEL_LIMIT).reader(), total, &speech_dir(), MODEL_SHA256, &mut progress)?;
    let whisper = if bundled(whisper) { Path::new(PROGRAM) } else { whisper };
    save_settings(&crate::settings::config_dir().join("settings.json"), whisper, &model)
}

/// Streams the model into a temporary file in `dir`, checks it against `sha256`, and only then
/// renames it into place: an interrupted or corrupt download never becomes the model, and
/// what's left of an earlier one is removed first.
pub fn install_model(mut source: impl Read, total: Option<u64>, dir: &Path, sha256: &str, mut progress: impl FnMut(u8)) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).context("couldn't create the speech folder")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".model-") && name.ends_with(".part") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let part = dir.join(format!(".model-{}.part", uuid::Uuid::new_v4().simple()));
    let result = (|| -> Result<PathBuf> {
        let mut file = std::fs::File::create_new(&part).context("couldn't save the speech model")?;
        let mut hasher = sha2::Sha256::new();
        let mut buffer = vec![0u8; 256 * 1024];
        let total = total.filter(|total| *total > 0).unwrap_or(MODEL_BYTES);
        let (mut received, mut shown) = (0u64, None);
        loop {
            let read = match source.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(anyhow::anyhow!("the download stopped ({error})")),
            };
            hasher.update(&buffer[..read]);
            file.write_all(&buffer[..read]).context("couldn't save the speech model (is the disk full?)")?;
            received += read as u64;
            // 100% means checked and in place.
            let percent = (received.saturating_mul(100) / total).min(99) as u8;
            if shown != Some(percent) {
                shown = Some(percent);
                progress(percent);
            }
        }
        file.sync_all()?;
        drop(file);
        let actual: String = hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
        anyhow::ensure!(actual.eq_ignore_ascii_case(sha256), "the downloaded speech model didn't match its checksum");
        let model = dir.join(MODEL_NAME);
        std::fs::rename(&part, &model).context("couldn't put the speech model in place")?;
        Ok(model)
    })();
    let _ = std::fs::remove_file(&part);
    result
}

/// Points `settings.json` at the runtime and model, keeping everything else in it (as the setup
/// scripts do; a language someone chose stays).
pub fn save_settings(config: &Path, whisper: &Path, model: &Path) -> Result<()> {
    let mut settings: serde_json::Value = match std::fs::read(config) {
        Ok(bytes) => serde_json::from_slice(&bytes).context("Lantern's settings.json isn't valid JSON")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(error) => return Err(error).context("couldn't read Lantern's settings"),
    };
    let object = settings.as_object_mut().context("Lantern's settings.json isn't a JSON object")?;
    object.insert("whisper".into(), whisper.to_string_lossy().into());
    object.insert("whisper_model".into(), model.to_string_lossy().into());
    if object.get("speech_language").and_then(|language| language.as_str()).is_none_or(str::is_empty) {
        object.insert("speech_language".into(), "auto".into());
    }
    crate::settings::private_write(config, serde_json::to_string_pretty(&settings)?.as_bytes()).context("couldn't save Lantern's settings")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lantern-speech-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sum(bytes: &[u8]) -> String {
        sha2::Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".part"))
            .collect()
    }

    #[test]
    fn a_checked_model_is_renamed_into_place() {
        let dir = folder();
        std::fs::write(dir.join(".model-old.part"), b"interrupted").unwrap();
        std::fs::write(dir.join(MODEL_NAME), b"previous").unwrap();
        let fake = vec![7u8; 600 * 1024];
        let mut seen = Vec::new();
        let model =
            install_model(std::io::Cursor::new(&fake), Some(fake.len() as u64), &dir, &sum(&fake), |percent| seen.push(percent)).unwrap();
        assert_eq!(model, dir.join(MODEL_NAME));
        assert_eq!(std::fs::read(&model).unwrap(), fake);
        assert!(leftovers(&dir).is_empty(), "the temporary file and an earlier one are gone");
        assert_eq!(seen.first(), Some(&42), "256 KiB of 600 KiB");
        assert_eq!(seen.last(), Some(&99), "100% only once it's checked and in place");
        assert!(seen.windows(2).all(|pair| pair[0] < pair[1]));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_corrupt_or_broken_download_never_becomes_the_model() {
        let dir = folder();
        let error = install_model(std::io::Cursor::new(b"tampered"), None, &dir, &sum(b"genuine"), |_| {}).unwrap_err();
        assert!(format!("{error:#}").contains("checksum"));
        assert!(!dir.join(MODEL_NAME).exists());
        assert!(leftovers(&dir).is_empty());

        struct Drops(usize);
        impl Read for Drops {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if self.0 == 0 {
                    return Err(std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset"));
                }
                self.0 -= 1;
                buffer[0] = 1;
                Ok(1)
            }
        }
        let error = install_model(Drops(3), Some(10), &dir, &sum(b"\x01\x01\x01"), |_| {}).unwrap_err();
        assert!(format!("{error:#}").contains("the download stopped"));
        assert!(!dir.join(MODEL_NAME).exists() && leftovers(&dir).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_point_at_the_runtime_and_keep_everything_else() {
        let dir = folder();
        let config = dir.join("config").join("settings.json");
        save_settings(&config, Path::new("whisper-cli"), &dir.join(MODEL_NAME)).unwrap();
        let written: Settings = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
        assert_eq!((written.whisper.as_str(), written.speech_language.as_str()), ("whisper-cli", "auto"));
        assert_eq!(written.whisper_model, dir.join(MODEL_NAME).to_string_lossy());

        std::fs::write(&config, r#"{"terminal":"kitty","speech_language":"de","whisper":"/gone/whisper-cli"}"#).unwrap();
        save_settings(&config, Path::new("/usr/bin/whisper-cli"), Path::new("/m/ggml-base.bin")).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
        assert_eq!(value["terminal"], "kitty");
        assert_eq!(value["speech_language"], "de", "a chosen language stays");
        assert_eq!(value["whisper"], "/usr/bin/whisper-cli");
        assert_eq!(value["whisper_model"], "/m/ggml-base.bin");

        std::fs::write(&config, "not json").unwrap();
        assert!(save_settings(&config, Path::new("whisper-cli"), Path::new("m")).is_err());
        assert_eq!(std::fs::read(&config).unwrap(), b"not json", "a broken file is left alone");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn whisper_is_found_beside_lantern_first() {
        let dir = folder();
        let (beside, on_path) = (dir.join("app"), dir.join("bin"));
        let program = format!("whisper-cli{}", std::env::consts::EXE_SUFFIX);
        for folder in [&beside, &on_path] {
            std::fs::create_dir_all(folder).unwrap();
            std::fs::write(folder.join(&program), b"").unwrap();
        }
        let path = std::env::join_paths([&on_path]).unwrap();
        assert_eq!(find_whisper("whisper-cli", Some(&beside), Some(&path)), Some(beside.join(&program)));
        assert_eq!(find_whisper("whisper-cli", None, Some(&path)), Some(on_path.join(&program)));
        // A path from the setup script that has gone falls back to the bundled runtime.
        let gone = dir.join("speech").join(&program);
        assert_eq!(find_whisper(&gone.to_string_lossy(), Some(&beside), None), Some(beside.join(&program)));
        let installed = on_path.join(&program);
        assert_eq!(find_whisper(&installed.to_string_lossy(), Some(&beside), None), Some(installed));
        assert_eq!(find_whisper("whisper-cli", Some(&dir), None), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn messages_say_what_to_do() {
        assert_eq!(progress(35), "Setting up dictation (148 MB, once)… 35%");
        assert_eq!(failed("the download stopped (reset)."), "Couldn't set up dictation: the download stopped (reset). Try again.");
        assert!(missing_runtime(true).contains("Reinstall Lantern"));
        let linux = missing_runtime(false);
        assert!(linux.contains("whisper-cli") && linux.contains("scripts/setup-dictation.sh"));
        assert!(!missing_runtime(true).contains("scripts/"), "no developer instructions on Windows");
    }
}
