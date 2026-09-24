//! A short list of what this Mac already has (tools, apps, local models, hardware), kept in
//! `toolkit.md` in Lantern's support folder and given to agents Lantern starts, so they use what's
//! here instead of installing it again.
//!
//! The list is rebuilt in the background when it's more than a day old (or on request), not
//! before every start. Everything under "## Your notes" is the user's and survives rebuilds.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use crate::hooks::support_dir;
use crate::launch::shell;
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

/// Scans the Mac and rewrites the list, keeping the user's notes.
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
        "# Tools on this Mac\n\nKept by October Lantern and given to agents it starts, so they use what's already here \
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

/// Which of these programs the user's login shell can run.
fn on_path<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let names: Vec<&str> = names.collect();
    let script = format!("for c in {}; do command -v \"$c\" >/dev/null 2>&1 && echo \"$c\"; done", names.join(" "));
    crate::run::output(Command::new(shell()).args(["-lic", &script]), Duration::from_secs(15))
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).filter(|l| names.contains(&l.as_str())).collect())
        .unwrap_or_default()
}

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

/// Ollama models, whisper.cpp model files, Hugging Face and LM Studio downloads.
fn models(ollama: bool) -> Vec<String> {
    let mut out = Vec::new();
    if ollama && let Ok(o) = crate::run::output(Command::new(shell()).args(["-lic", "ollama list 2>/dev/null"]), Duration::from_secs(10)) {
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

fn list(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

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
