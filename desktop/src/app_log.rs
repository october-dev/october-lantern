//! The app's own log, for Send Logs (Swift `AppLog`): logs/app.log in Lantern's data folder (the
//! engine's is engine.log). A line per event worth knowing about afterwards, never conversations,
//! folders or keys. Over 1 MB it starts again, the old one kept as app-previous.log.
use std::io::Write;

pub fn write(line: impl std::fmt::Display) {
    let now = time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap_or_default();
    let logs = crate::settings::data_dir().join("logs");
    let log = logs.join("app.log");
    let _ = std::fs::create_dir_all(&logs);
    if std::fs::metadata(&log).is_ok_and(|m| m.len() > 1_000_000) {
        let _ = std::fs::rename(&log, logs.join("app-previous.log"));
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
        let _ = writeln!(file, "{now} {line}");
    }
}
