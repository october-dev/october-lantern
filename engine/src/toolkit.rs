//! A short list of what this computer already has (tools, apps, local models, hardware), kept in
//! `toolkit.md` in Lantern's support folder and given to agents Lantern starts, so they use what's
//! here instead of installing it again.
//!
//! The list is rebuilt in the background when it's more than a day old (or on request), not
//! before every start. Everything under "## Your notes" is the user's and survives rebuilds.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use crate::hooks::support_dir;
use crate::transcripts::home;

pub const NOTES_HEADING: &str = "## Your notes";

pub fn path() -> PathBuf {
    support_dir().join("toolkit.md")
}

/// The list as it's given to agents, or `None` when there isn't one yet.
pub fn text() -> Option<String> {
    std::fs::read_to_string(path()).ok().filter(|t| !t.trim().is_empty())
}

/// Rebuilds the list when it's missing or more than a day old.
pub fn refresh_if_stale() {
    let fresh = std::fs::metadata(path())
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < Duration::from_secs(24 * 3600));
    if !fresh {
        let _ = refresh();
    }
}

/// Scans this computer and rewrites the list, keeping the user's notes.
pub fn refresh() -> anyhow::Result<PathBuf> {
    let notes = std::fs::read_to_string(path()).ok().and_then(|t| notes_of(&t).map(String::from)).unwrap_or_default();
    let body = render(&scan(), &notes);
    std::fs::create_dir_all(support_dir())?;
    crate::hooks::write_atomic(&path(), body.as_bytes())?;
    Ok(path())
}

#[derive(Default)]
pub struct Found {
    pub machine: Option<String>,
    /// (group, names) in display order.
    pub tools: Vec<(&'static str, Vec<String>)>,
    pub models: Vec<String>,
    pub apps: Vec<String>,
}

/// What's below the notes heading (on a line of its own).
pub fn notes_of(text: &str) -> Option<&str> {
    let start = text.lines().position(|l| l.trim() == NOTES_HEADING)?;
    let offset: usize = text.lines().take(start + 1).map(|l| l.len() + 1).sum();
    Some(text.get(offset..).unwrap_or("").trim_start_matches('\n'))
}

pub fn render(found: &Found, notes: &str) -> String {
    let today = crate::model::iso(crate::hooks::now_ms());
    let mut out = format!(
        "# Tools on this computer\n\nKept by October Lantern and given to agents it starts, so they use what's already here \
         instead of installing it again. Updated {}. Lantern rewrites everything above the notes heading; what you write under \
         it is kept.\n\n",
        &today[..10.min(today.len())]
    );
    if let Some(m) = &found.machine {
        out.push_str(&format!("- Machine: {m}\n"));
    }
    for (group, names) in &found.tools {
        if !names.is_empty() {
            out.push_str(&format!("- {group}: {}\n", names.join(", ")));
        }
    }
    if !found.models.is_empty() {
        out.push_str(&format!("- Local AI models: {}\n", found.models.join("; ")));
    }
    if !found.apps.is_empty() {
        out.push_str(&format!("- Apps: {}\n", found.apps.join("; ")));
    }
    out.push_str(&format!("\n{NOTES_HEADING}\n\n{notes}"));
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Command-line tools worth knowing about, by group.
const TOOLS: &[(&str, &[&str])] = &[
    ("Media", &["ffmpeg", "ffprobe", "magick", "convert", "sox", "yt-dlp", "exiftool", "HandBrakeCLI", "gifski", "tesseract"]),
    ("Documents", &["pandoc", "qpdf", "pdftotext", "mutool", "gs", "libreoffice", "soffice", "typst", "tectonic", "pdflatex"]),
    ("Speech and AI", &["whisper", "whisper-cli", "whisper-cpp", "ollama", "llm", "lms", "mlx_lm.generate"]),
    ("Languages", &["python3", "uv", "node", "bun", "deno", "go", "cargo", "swift", "java", "ruby", "php"]),
    ("Developer", &["git", "gh", "docker", "jq", "rg", "fd", "sqlite3", "psql", "redis-cli", "xcodebuild", "tmux", "brew"]),
    ("Cloud", &["aws", "gcloud", "az", "vercel", "netlify", "fly", "wrangler", "supabase", "kubectl", "terraform"]),
];

/// Apps agents can script or drive, with where their scripting lives when it isn't obvious.
#[cfg(target_os = "macos")]
const APPS: &[(&str, &str)] = &[
    ("DaVinci Resolve", "scripting API in /Library/Application Support/Blackmagic Design/DaVinci Resolve/Developer/Scripting"),
    ("Blender", "Python: Blender.app/Contents/MacOS/Blender --background --python script.py"),
    ("Unity Hub", "editors in /Applications/Unity/Hub/Editor; batch mode with -batchmode -executeMethod"),
    ("Final Cut Pro", "FCPXML import/export"),
    ("Logic Pro", ""),
    ("GarageBand", ""),
    ("Adobe Photoshop", "ExtendScript / UXP"),
    ("Adobe Premiere Pro", "ExtendScript / UXP"),
    ("Adobe After Effects", "ExtendScript"),
    ("Adobe Illustrator", "ExtendScript"),
    ("Figma", ""),
    ("Sketch", ""),
    ("Affinity Photo 2", ""),
    ("Affinity Designer 2", ""),
    ("Keynote", "AppleScript"),
    ("Pages", "AppleScript"),
    ("Numbers", "AppleScript"),
    ("Microsoft Word", "AppleScript"),
    ("Microsoft Excel", "AppleScript"),
    ("Microsoft PowerPoint", "AppleScript"),
    ("LibreOffice", "soffice --headless --convert-to"),
    ("OBS", ""),
    ("Audacity", ""),
    ("Ableton Live 12 Suite", ""),
    ("Cinema 4D", ""),
    ("TouchDesigner", ""),
    ("Xcode", "xcodebuild"),
    ("Android Studio", ""),
    ("Docker", ""),
    ("Screen Studio", ""),
    ("Preview", ""),
];

pub(crate) fn scan() -> Found {
    let mut found = Found { machine: machine(), ..Default::default() };
    let installed = on_path(TOOLS.iter().flat_map(|(_, names)| names.iter().copied()));
    for (group, names) in TOOLS {
        found.tools.push((group, names.iter().filter(|n| installed.iter().any(|i| i == *n)).map(|n| n.to_string()).collect()));
    }
    found.models = models(installed.iter().any(|i| i == "ollama"));
    found.apps = apps();
    found
}

/// Which of these programs are on `PATH`.
#[cfg(windows)]
fn on_path<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    names.filter(|n| crate::platform::find_on_path(n).is_some()).map(String::from).collect()
}

/// Executables on the same cached environment/PATH as agent launches.
#[cfg(unix)]
fn on_path<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    names.filter(|name| crate::environment::login().find(name).is_some()).map(String::from).collect()
}

#[cfg(target_os = "macos")]
fn machine() -> Option<String> {
    let sysctl = |key: &str| {
        crate::run::output(Command::new("/usr/sbin/sysctl").args(["-n", key]), Duration::from_secs(3))
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let chip = sysctl("machdep.cpu.brand_string")?;
    let memory = sysctl("hw.memsize").and_then(|m| m.parse::<u64>().ok()).map(|b| format!(", {} GB memory", b / (1 << 30)));
    let os = crate::run::output(Command::new("/usr/bin/sw_vers").arg("-productVersion"), Duration::from_secs(3))
        .ok()
        .map(|o| format!(", macOS {}", String::from_utf8_lossy(&o.stdout).trim()));
    Some(format!("{chip}{}{}", memory.unwrap_or_default(), os.unwrap_or_default()))
}

#[cfg(target_os = "linux")]
fn machine() -> Option<String> {
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                line.strip_prefix("model name").and_then(|line| line.split_once(':')).map(|(_, value)| value.trim().to_owned())
            })
        })
        .unwrap_or_else(|| std::env::consts::ARCH.into());
    let memory = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                line.strip_prefix("MemTotal:").and_then(|value| value.split_whitespace().next()).and_then(|value| value.parse::<u64>().ok())
            })
        })
        .map(|kb| format!(", {} GB memory", kb / (1024 * 1024)))
        .unwrap_or_default();
    Some(format!("{cpu}{memory}, {}", sysinfo::System::long_os_version().unwrap_or_else(|| "Linux".into())))
}

#[cfg(windows)]
fn machine() -> Option<String> {
    let mut sys = sysinfo::System::new();
    sys.refresh_cpu_list(sysinfo::CpuRefreshKind::nothing());
    sys.refresh_memory();
    let cpu =
        sys.cpus().first().map(|c| c.brand().trim().to_string()).filter(|b| !b.is_empty()).unwrap_or_else(|| std::env::consts::ARCH.into());
    let memory = format!(", {} GB memory", sys.total_memory() / (1 << 30));
    Some(format!("{cpu}{memory}, {}", sysinfo::System::long_os_version().unwrap_or_else(|| "Windows".into())))
}

/// Ollama models, whisper.cpp model files, Hugging Face and LM Studio downloads.
fn models(ollama: bool) -> Vec<String> {
    let mut out = Vec::new();
    #[cfg(unix)]
    let list_ollama = || crate::run::output(crate::environment::login().command("ollama")?.arg("list"), Duration::from_secs(10));
    #[cfg(windows)]
    let list_ollama = || {
        let bin = crate::platform::find_on_path("ollama").ok_or_else(|| anyhow::anyhow!("ollama isn't on PATH"))?;
        crate::run::output(Command::new(bin).arg("list"), Duration::from_secs(10))
    };
    if ollama && let Ok(o) = list_ollama() {
        let names: Vec<String> = String::from_utf8_lossy(&o.stdout)
            .lines()
            .skip(1)
            .filter_map(|l| l.split_whitespace().next().map(String::from))
            .take(12)
            .collect();
        if !names.is_empty() {
            out.push(format!("Ollama: {}", names.join(", ")));
        }
    }
    let home = home();
    let mut whisper: Vec<String> = Vec::new();
    for dir in ["models", ".cache/whisper", "whisper.cpp/models", ".whisper", "Library/Application Support/whisper"] {
        for f in list(&home.join(dir)) {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if (name.starts_with("ggml-") && name.ends_with(".bin")) || name.ends_with(".pt") {
                whisper.push(tilde(&f));
            }
        }
    }
    if !whisper.is_empty() {
        whisper.truncate(8);
        out.push(format!("Whisper: {}", whisper.join(", ")));
    }
    let hf: Vec<String> = list(&home.join(".cache/huggingface/hub"))
        .iter()
        .filter_map(|p| p.file_name()?.to_str()?.strip_prefix("models--").map(|m| m.replacen("--", "/", 1)))
        .take(12)
        .collect();
    if !hf.is_empty() {
        out.push(format!("Hugging Face cache (~/.cache/huggingface/hub): {}", hf.join(", ")));
    }
    let lms: Vec<String> = [".lmstudio/models", ".cache/lm-studio/models"]
        .iter()
        .flat_map(|d| list(&home.join(d)))
        .flat_map(|publisher| list(&publisher))
        .filter_map(|m| m.file_name().map(|n| n.to_string_lossy().into_owned()))
        .take(12)
        .collect();
    if !lms.is_empty() {
        out.push(format!("LM Studio: {}", lms.join(", ")));
    }
    out
}

/// Notable apps in /Applications (and one folder down, where some apps live), and ~/Applications.
#[cfg(target_os = "macos")]
fn apps() -> Vec<String> {
    let home = home();
    let roots = [PathBuf::from("/Applications"), home.join("Applications")];
    let mut bundles: Vec<String> = Vec::new();
    for root in &roots {
        for entry in list(root) {
            if entry.extension().is_some_and(|e| e == "app") {
                bundles.push(stem(&entry));
            } else if entry.is_dir() {
                bundles.extend(list(&entry).iter().filter(|e| e.extension().is_some_and(|x| x == "app")).map(|e| stem(e)));
            }
        }
    }
    if Path::new("/System/Applications/Preview.app").exists() {
        bundles.push("Preview".into());
    }
    APPS.iter()
        .filter(|(name, _)| bundles.iter().any(|b| b == name))
        .map(|(name, note)| if note.is_empty() { name.to_string() } else { format!("{name} ({note})") })
        .collect()
}

#[cfg(target_os = "linux")]
fn apps() -> Vec<String> {
    let mut roots = vec![crate::platform::data_dir("october-lantern").parent().unwrap_or(Path::new("/usr/share")).join("applications")];
    roots.extend(
        std::env::var("XDG_DATA_DIRS")
            .unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
            .split(':')
            .filter(|dir| !dir.is_empty())
            .map(|dir| PathBuf::from(dir).join("applications")),
    );
    let mut apps: Vec<String> = roots
        .iter()
        .flat_map(|root| list(root))
        .filter(|path| path.extension().is_some_and(|ext| ext == "desktop"))
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter(|text| !text.lines().any(|line| line == "Hidden=true" || line == "NoDisplay=true"))
        .filter_map(|text| text.lines().find_map(|line| line.strip_prefix("Name=").map(str::to_string)))
        .collect();
    apps.sort();
    apps.dedup();
    apps.truncate(60);
    apps
}

/// Start menu shortcuts, for the user and for everyone.
#[cfg(windows)]
fn apps() -> Vec<String> {
    let programs = Path::new("Microsoft").join("Windows").join("Start Menu").join("Programs");
    let roots: Vec<PathBuf> =
        ["APPDATA", "ProgramData"].iter().filter_map(std::env::var_os).map(|d| PathBuf::from(d).join(&programs)).collect();
    let mut apps: Vec<String> = roots
        .iter()
        .flat_map(|root| {
            let entries = list(root);
            let nested: Vec<PathBuf> = entries.iter().filter(|e| e.is_dir()).flat_map(|d| list(d)).collect();
            entries.into_iter().chain(nested)
        })
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .filter(|name| !name.to_ascii_lowercase().contains("uninstall"))
        .collect();
    apps.sort();
    apps.dedup();
    apps.truncate(60);
    apps
}

fn list(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn tilde(p: &Path) -> String {
    let home = home();
    match p.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}
