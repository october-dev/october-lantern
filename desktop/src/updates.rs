//! Releases use native, checksum-verified bundles (a tar.gz on Linux, a zip on Windows, both
//! unpacked with the system's `tar`). Installation is an explicit UI action: the new version goes
//! into its own folder next to the old ones, and only after it proves it can talk to its engine.
use crate::settings::{data_dir, private_write};
use anyhow::{Context, Result};
use serde_json::Value;
use sha2::Digest;
use std::path::{Path, PathBuf};
use std::process::Command;
pub(crate) const REPO: &str = match option_env!("LANTERN_RELEASES_REPO") {
    Some(repo) => repo,
    // Releases are published next to the Mac's, in the public releases repository.
    None => "harshsaver/october-lantern-releases",
};
#[cfg(not(windows))]
const FILES: &[&str] = &[
    "october-lantern",
    "lantern-engine",
    "october-bus",
    "october-lantern.png",
    "LICENSE",
    "OCTOBER-BUS-LICENSE",
    "OCTOBER-BUS-COMMIT",
    "lantern-gnome-extension.zip",
];
#[cfg(windows)]
const FILES: &[&str] = &[
    "october-lantern.exe",
    "lantern-engine.exe",
    "october-bus.exe",
    "october-lantern.png",
    "LICENSE",
    "OCTOBER-BUS-LICENSE",
    "OCTOBER-BUS-COMMIT",
];
/// Files a bundle may also carry (and that 0.6.2's updater would refuse, so releases don't ship
/// them in the zip yet): dictation's speech runtime and its license. Anything else is refused.
#[cfg(not(windows))]
const OPTIONAL: &[&str] = &[];
#[cfg(windows)]
const OPTIONAL: &[&str] = &["whisper-cli.exe", "WHISPER-LICENSE"];
/// The programs in a bundle, checked to be native executables before anything runs.
const PROGRAMS: [&str; 3] = ["october-lantern", "lantern-engine", "october-bus"];
/// Programs a bundle may carry, checked the same way when it does.
const OPTIONAL_PROGRAMS: [&str; 1] = ["whisper-cli"];
/// Lantern's own download site: the same release files as GitHub, for networks that block
/// GitHub's file host. `latest.json` names the newest Windows bundle and its checksum.
const SITE: &str = "https://lantern.october.dev/download/";
#[derive(Clone)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub checksum: Checksum,
}
/// Where a bundle's SHA-256 comes from: the site's `latest.json` says it, GitHub serves it as a
/// file beside the bundle.
#[derive(Clone)]
pub enum Checksum {
    Known(String),
    At(String),
}
impl Release {
    pub fn is_newer(&self) -> bool {
        semver::Version::parse(self.version.trim_start_matches('v'))
            .ok()
            .zip(semver::Version::parse(env!("CARGO_PKG_VERSION")).ok())
            .is_some_and(|(release, current)| release > current)
    }
}
/// The newest release: from Lantern's site (Windows on x86_64), else from GitHub.
pub fn check() -> Result<Release> {
    if cfg!(all(windows, target_arch = "x86_64"))
        && let Ok(release) = check_site()
    {
        return Ok(release);
    }
    check_github()
}
fn check_site() -> Result<Release> {
    let mut response = crate::account::agent().get(&format!("{SITE}latest.json")).header("User-Agent", "October-Lantern").call()?;
    let value: Value = response.body_mut().with_config().limit(64 * 1024).read_json()?;
    site_release(&value)
}
fn site_release(value: &Value) -> Result<Release> {
    let windows = &value["windows"];
    let version = windows["version"].as_str().context("no Windows version")?.to_owned();
    anyhow::ensure!(safe_version(&version), "invalid release version");
    let update = &windows["update"];
    anyhow::ensure!(update["file"] == asset_name().as_str(), "the site has no update bundle for this system");
    let url = update["url"].as_str().context("no update bundle")?.to_owned();
    anyhow::ensure!(trusted_url(&url), "update bundle is outside Lantern's site");
    let sha256 = update["sha256"].as_str().context("no update checksum")?.to_owned();
    Ok(Release { version, url, checksum: Checksum::Known(sha256) })
}
fn check_github() -> Result<Release> {
    let mut response = crate::account::agent()
        .get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("User-Agent", "October-Lantern")
        .call()?;
    let value: Value = response.body_mut().with_config().limit(2 * 1024 * 1024).read_json()?;
    let version = value["tag_name"].as_str().context("release version missing")?.to_owned();
    anyhow::ensure!(safe_version(&version), "invalid release version");
    let name = asset_name();
    let assets = value["assets"].as_array().context("No release assets")?;
    let find = |name: &str| -> Result<String> {
        let url = assets
            .iter()
            .find(|asset| asset["name"] == name)
            .and_then(|asset| asset["browser_download_url"].as_str())
            .context(format!("The latest published release has no {} build yet", system_name()))?;
        anyhow::ensure!(trusted_url(url), "release download is outside Lantern's release repository");
        Ok(url.into())
    };
    Ok(Release { version, url: find(&name)?, checksum: Checksum::At(find(&format!("{name}.sha256"))?) })
}
fn asset_name() -> String {
    if cfg!(windows) {
        format!("October-Lantern-windows-{}.zip", std::env::consts::ARCH)
    } else {
        format!("October-Lantern-linux-{}.tar.gz", std::env::consts::ARCH)
    }
}
pub fn system_name() -> &'static str {
    if cfg!(windows) { "Windows" } else { "Linux" }
}
fn safe_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 80
        && version.bytes().all(|byte| byte.is_ascii_alphanumeric() || b".-_".contains(&byte))
        && version != "."
        && version != ".."
}
pub(crate) fn trusted_url(url: &str) -> bool {
    url.starts_with(&format!("https://github.com/{REPO}/releases/download/")) || url.starts_with(SITE)
}
fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}
/// The system's `tar`: Windows 10 and later ship bsdtar, which also reads zip files. Its own
/// copy, not whichever `tar` is first on PATH (Git's can't).
pub(crate) fn tar() -> Command {
    if cfg!(windows) {
        let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        crate::os::command(system.join("System32").join("tar.exe"))
    } else {
        Command::new("tar")
    }
}
pub fn install(release: Release) -> Result<PathBuf> {
    let checksum_url = match &release.checksum {
        Checksum::At(url) => Some(url),
        Checksum::Known(_) => None,
    };
    anyhow::ensure!(
        safe_version(&release.version) && trusted_url(&release.url) && checksum_url.is_none_or(|url| trusted_url(url)),
        "invalid release source"
    );
    let agent = crate::account::agent();
    let checksum = match &release.checksum {
        Checksum::Known(sha256) => sha256.clone(),
        Checksum::At(url) => agent.get(url).call()?.body_mut().with_config().limit(4096).read_to_string()?,
    };
    let bytes = agent.get(&release.url).call()?.body_mut().with_config().limit(128 * 1024 * 1024).read_to_vec()?;
    verify(&bytes, &checksum)?;
    let root = data_dir().join("releases");
    std::fs::create_dir_all(&root)?;
    let work = root.join(format!(".update-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&work)?;
    set_mode(&work, 0o700)?;
    let result = (|| -> Result<PathBuf> {
        let archive = work.join(if cfg!(windows) { "bundle.zip" } else { "bundle.tar.gz" });
        private_write(&archive, &bytes)?;
        let listing = tar().arg("-tf").arg(&archive).output()?;
        anyhow::ensure!(listing.status.success(), "invalid release archive");
        let names = String::from_utf8(listing.stdout)?;
        let names: Vec<&str> = names.lines().collect();
        anyhow::ensure!(archive_matches(&names, FILES, OPTIONAL), "unexpected archive contents");
        let types = tar().arg("-tvf").arg(&archive).output()?;
        anyhow::ensure!(
            types.status.success() && String::from_utf8(types.stdout)?.lines().all(|line| line.starts_with('-')),
            "release archive contains links or non-files"
        );
        let mut unpack = tar();
        unpack.arg("-xf").arg(&archive);
        if cfg!(unix) {
            unpack.args(["--no-same-owner", "--no-same-permissions"]);
        }
        let output = unpack.arg("-C").arg(&work).output()?;
        anyhow::ensure!(output.status.success(), "could not unpack release");
        for name in PROGRAMS.into_iter().chain(OPTIONAL_PROGRAMS.into_iter().filter(|name| work.join(exe(name)).exists())) {
            validate_binary(&std::fs::read(work.join(exe(name)))?)?;
            set_mode(&work.join(exe(name)), 0o755)?;
        }
        let output = crate::os::command(work.join(exe("october-lantern")))
            .arg("--check-sidecar")
            .env("LANTERN_ENGINE", work.join(exe("lantern-engine")))
            .output()?;
        anyhow::ensure!(output.status.success(), "updated desktop and engine are incompatible");
        std::fs::remove_file(&archive)?;
        let dest = root.join(format!("{}-{}", release.version, uuid::Uuid::new_v4().simple()));
        std::fs::rename(&work, &dest)?;
        install_launcher(&dest)?;
        if crate::settings::autostart_enabled() {
            crate::settings::autostart_executable(true, &dest.join(exe("october-lantern")))?;
        }
        Ok(dest.join(exe("october-lantern")))
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}
/// An archive's listing holds every `required` file once, and otherwise only `optional` ones,
/// each at most once: nothing else, and no folders.
pub(crate) fn archive_matches(names: &[&str], required: &[&str], optional: &[&str]) -> bool {
    let once = |name: &&str| names.iter().filter(|entry| *entry == name).count() == 1;
    required.iter().all(once) && names.iter().all(|name| (required.contains(name) || optional.contains(name)) && once(name))
}
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(windows)]
    let _ = (path, mode);
    Ok(())
}
/// A program built for this system and processor: ELF on Linux, PE on Windows.
pub(crate) fn validate_binary(bytes: &[u8]) -> Result<()> {
    if cfg!(windows) { validate_pe(bytes) } else { validate_elf(bytes) }
}
fn validate_elf(bytes: &[u8]) -> Result<()> {
    let machine = if cfg!(target_arch = "aarch64") { 183u16 } else { 62u16 };
    anyhow::ensure!(
        bytes.len() > 20
            && bytes.starts_with(b"\x7fELF")
            && bytes[4] == 2
            && bytes[5] == 1
            && u16::from_le_bytes([bytes[18], bytes[19]]) == machine,
        "release binary is not native to this Linux architecture"
    );
    Ok(())
}
fn validate_pe(bytes: &[u8]) -> Result<()> {
    let machine = if cfg!(target_arch = "aarch64") { 0xaa64u16 } else { 0x8664u16 };
    let header = bytes.get(0x3c..0x40).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let native = bytes.starts_with(b"MZ")
        && header
            .and_then(|at| bytes.get(at..at + 6))
            .is_some_and(|pe| pe[..4] == *b"PE\0\0" && u16::from_le_bytes([pe[4], pe[5]]) == machine);
    anyhow::ensure!(native, "release program is not native to this Windows architecture");
    Ok(())
}
pub(crate) fn verify(bytes: &[u8], checksum: &str) -> Result<()> {
    let expected = checksum.split_whitespace().next().context("missing checksum")?;
    anyhow::ensure!(expected.len() == 64 && expected.bytes().all(|byte| byte.is_ascii_hexdigit()), "invalid checksum");
    let actual: String = sha2::Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    anyhow::ensure!(actual.eq_ignore_ascii_case(expected), "release checksum does not match; nothing installed");
    Ok(())
}
#[cfg(not(windows))]
pub fn install_launcher(bundle: &Path) -> Result<()> {
    use crate::settings::xdg;
    let executable = bundle.join("october-lantern");
    anyhow::ensure!(executable.is_file(), "bundle has no app");
    let escaped =
        executable.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"").replace('`', "\\`").replace('$', "\\$").replace('%', "%%");
    let icon = xdg("XDG_DATA_HOME", ".local/share").join("icons/hicolor/256x256/apps/dev.october.lantern.png");
    private_write(&icon, &std::fs::read(bundle.join("october-lantern.png"))?)?;
    private_write(&xdg("XDG_DATA_HOME", ".local/share").join("applications/dev.october.lantern.desktop"),
        format!("[Desktop Entry]\nType=Application\nName=October Lantern\nComment=Stay close to your agents\nExec=\"{escaped}\"\nIcon=dev.october.lantern\nTerminal=false\nCategories=Development;Utility;\nStartupWMClass=dev.october.lantern\n").as_bytes())
}
/// The Start menu shortcut (`October Lantern`), pointing at this bundle's app. Windows makes
/// shortcuts through a COM object, reached here through PowerShell; the paths travel as
/// environment variables, never as script text.
#[cfg(windows)]
pub fn install_launcher(bundle: &Path) -> Result<()> {
    let executable = bundle.join("october-lantern.exe");
    anyhow::ensure!(executable.is_file(), "bundle has no app");
    let link = start_menu_shortcut();
    std::fs::create_dir_all(link.parent().context("no Start menu folder")?)?;
    let script = "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:LANTERN_LINK); \
        $s.TargetPath = $env:LANTERN_TARGET; $s.WorkingDirectory = $env:LANTERN_DIR; $s.IconLocation = $env:LANTERN_TARGET; \
        $s.Description = 'Stay close to your agents'; $s.Save()";
    let output = crate::os::command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("LANTERN_LINK", &link)
        .env("LANTERN_TARGET", &executable)
        .env("LANTERN_DIR", bundle)
        .output()
        .context("creating the Start menu shortcut")?;
    anyhow::ensure!(output.status.success(), "Windows couldn't create the Start menu shortcut");
    Ok(())
}
#[cfg(windows)]
pub fn start_menu_shortcut() -> PathBuf {
    crate::settings::xdg("APPDATA", "AppData/Roaming").join(r"Microsoft\Windows\Start Menu\Programs\October Lantern.lnk")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_paths_and_foreign_release_urls() {
        for version in ["../x", "/tmp/x", "..", "a b", ""] {
            assert!(!safe_version(version));
        }
        assert!(safe_version("v0.4.3"));
        assert!(!trusted_url("https://github.com/attacker/repo/releases/download/x"));
    }
    #[test]
    fn never_offers_same_version_or_downgrades() {
        let release = |version: &str| Release { version: version.into(), url: String::new(), checksum: Checksum::At(String::new()) };
        assert!(!release("v0.1.0").is_newer());
        assert!(!release(env!("CARGO_PKG_VERSION")).is_newer());
        assert!(release("v1.0.0").is_newer());
        assert!(!release("invalid").is_newer());
    }
    #[test]
    fn the_sites_latest_json_names_this_systems_bundle_and_checksum() {
        let latest = |file: &str, url: &str| {
            serde_json::json!({"version": "v0.6.7", "windows": {"version": "v0.6.6", "url": "https://lantern.october.dev/download/windows",
                "update": {"url": url, "file": file, "sha256": "ab".repeat(32)}}})
        };
        let ours = latest(&asset_name(), "https://lantern.october.dev/download/windows-update");
        let release = site_release(&ours).unwrap();
        assert_eq!(release.version, "v0.6.6", "the Windows version, not the newest release's");
        assert!(matches!(release.checksum, Checksum::Known(ref sha) if sha.len() == 64));
        assert!(
            site_release(&latest("October-Lantern-windows-aarch64.zip", "https://lantern.october.dev/download/windows-update")).is_err()
        );
        assert!(site_release(&latest(&asset_name(), "https://example.com/evil.zip")).is_err());
        assert!(site_release(&serde_json::json!({"windows": {"version": "v0.6.6"}})).is_err(), "no bundle yet: GitHub instead");
    }
    #[test]
    fn checks_content_before_unpacking() {
        let checksum: String = sha2::Sha256::digest(b"valid").iter().map(|byte| format!("{byte:02x}")).collect();
        assert!(verify(b"valid", &format!("{checksum} archive.tar.gz")).is_ok());
        assert!(verify(b"tampered", &checksum).is_err());
        assert!(verify(b"valid", "not-a-checksum").is_err());
        assert!(validate_elf(b"MZ foreign binary").is_err());
        assert!(validate_pe(b"\x7fELF foreign binary").is_err());
    }
    #[test]
    fn a_bundle_has_its_core_files_and_only_known_extras() {
        let core = ["a.exe", "LICENSE"];
        let extras = ["whisper-cli.exe", "WHISPER-LICENSE"];
        assert!(archive_matches(&["LICENSE", "a.exe"], &core, &extras), "the core files alone (as 0.6.2 expects)");
        assert!(archive_matches(&["a.exe", "LICENSE", "whisper-cli.exe", "WHISPER-LICENSE"], &core, &extras), "with the extras");
        assert!(archive_matches(&["a.exe", "LICENSE", "whisper-cli.exe"], &core, &extras), "with some of them");
        assert!(!archive_matches(&["a.exe"], &core, &extras), "a core file missing");
        assert!(!archive_matches(&["a.exe", "LICENSE", "evil.dll"], &core, &extras), "an unknown file");
        assert!(!archive_matches(&["a.exe", "LICENSE", "sub/"], &core, &extras), "a folder");
        assert!(!archive_matches(&["a.exe", "LICENSE", "a.exe"], &core, &extras), "a duplicate");
        assert!(!archive_matches(&["a.exe", "LICENSE", "whisper-cli.exe", "whisper-cli.exe"], &core, &extras), "a duplicate extra");
        // This version's own lists: the zip 0.6.2 accepts is still accepted, extras or not.
        assert!(archive_matches(FILES, FILES, OPTIONAL));
        let with: Vec<&str> = FILES.iter().chain(OPTIONAL).copied().collect();
        assert!(archive_matches(&with, FILES, OPTIONAL));
    }

    #[test]
    fn windows_programs_must_match_the_processor() {
        let pe = |machine: u16| {
            let mut bytes = vec![0u8; 0x48];
            bytes[..2].copy_from_slice(b"MZ");
            bytes[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
            bytes[0x40..0x44].copy_from_slice(b"PE\0\0");
            bytes[0x44..0x46].copy_from_slice(&machine.to_le_bytes());
            bytes
        };
        let native = if cfg!(target_arch = "aarch64") { 0xaa64 } else { 0x8664 };
        assert!(validate_pe(&pe(native)).is_ok());
        assert!(validate_pe(&pe(0x014c)).is_err(), "32-bit x86");
        assert!(validate_pe(b"MZ").is_err());
    }
}
