//! October Lantern for Windows and Linux: the Mac app's pill and panel, in Rust. Discovery,
//! targeting and agent delivery stay in the Rust engine (the sidecar).
// Release builds on Windows are windowed apps: no console window behind Lantern.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod account;
mod app;
mod model;
mod october_ai;
mod os;
mod settings;
mod shell;
mod sidecar;
mod speech;
mod state;
mod ui;
mod updates;

use settings::Settings;
use sidecar::Sidecar;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const HELP: &str = "October Lantern
  (no option)          Start Lantern, or show the running one
  --new-session        Open New Session
  --point-ask          Start Point & Ask
  --ask                Open Ask Lantern
  --connections        Open October (account and October Desktop)
  --settings           Open Settings
  --welcome            Open the welcome guide
  --check-sidecar      Verify the Rust engine
  --check-desktop      Verify desktop portals
  --install-launcher   Install the application launcher
  --fixture FILE.json [--out FILE.png] [--scale 1|2]   Render one surface from a saved model";

/// What a launch asks for, as the running Lantern understands it (tray, shortcuts and a second
/// launch use the same names).
fn activation(args: &[String]) -> &'static str {
    let has = |flag: &str| args.iter().any(|arg| arg == flag);
    if has("--new-session") {
        "new"
    } else if has("--point-ask") {
        "capture"
    } else if has("--ask") {
        "ask"
    } else if has("--connections") {
        "connections"
    } else if has("--settings") {
        "settings"
    } else if has("--welcome") || has("--help-page") {
        "welcome"
    } else {
        "show"
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    // After an update: wait for the old Lantern to exit before taking over.
    if let Some(index) = args.iter().position(|arg| arg == "--restart-after") {
        let pid: u32 = args.get(index + 1).ok_or_else(|| anyhow::anyhow!("missing restart process"))?.parse()?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while os::process_alive(pid) {
            anyhow::ensure!(Instant::now() < deadline, "Previous Lantern process did not exit");
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    if args.iter().any(|arg| arg == "--help") {
        println!("{HELP}");
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--check-desktop") {
        println!("{}", os::diagnostics()?);
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--install-launcher") {
        updates::install_launcher(std::env::current_exe()?.parent().ok_or_else(|| anyhow::anyhow!("no bundle folder"))?)?;
        println!("Application launcher installed");
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--check-sidecar") {
        let engine = Sidecar::start()?;
        let event = engine.events.recv_timeout(Duration::from_secs(10))??;
        anyhow::ensure!(event["type"] == "hello" && event["protocol"] == 3, "unsupported engine protocol");
        println!("Sidecar connected: protocol {}, engine {}", event["protocol"], event["version"]);
        return Ok(());
    }
    if let Some(index) = args.iter().position(|arg| arg == "--fixture") {
        let fixture = args.get(index + 1).ok_or_else(|| anyhow::anyhow!("--fixture requires a JSON path"))?;
        let out = match args.iter().position(|arg| arg == "--out") {
            Some(index) => PathBuf::from(args.get(index + 1).ok_or_else(|| anyhow::anyhow!("--out requires a PNG path"))?),
            None => PathBuf::from(fixture).with_extension("png"),
        };
        let scale = match args.iter().position(|arg| arg == "--scale") {
            Some(index) => args.get(index + 1).ok_or_else(|| anyhow::anyhow!("--scale requires 1 or 2"))?.parse()?,
            None => 1.0,
        };
        return app::fixture::run(std::path::Path::new(fixture), out, scale);
    }
    let action = activation(&args);
    // One Lantern: a second launch hands its request to the running one.
    if os::show_existing(action) {
        return Ok(());
    }
    let settings = Settings::load().unwrap_or_default();
    app::run(&settings.terminal, (action != "show").then_some(action))
}

#[cfg(test)]
mod tests {
    use super::activation;

    #[test]
    fn launch_options_map_to_what_the_running_lantern_opens() {
        let args = |flag: &str| vec!["october-lantern".to_string(), flag.to_string()];
        assert_eq!(activation(&args("--new-session")), "new");
        assert_eq!(activation(&args("--point-ask")), "capture");
        assert_eq!(activation(&args("--settings")), "settings");
        assert_eq!(activation(&args("--help-page")), "welcome");
        assert_eq!(activation(&["october-lantern".to_string()]), "show");
        // The old --shell (the new app before it was the default) still just starts Lantern.
        assert_eq!(activation(&args("--shell")), "show");
    }
}
