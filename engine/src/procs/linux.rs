//! Linux process identity and terminal ownership from procfs. No `ps` subprocess is needed.
use super::*;
use std::os::unix::fs::MetadataExt;

struct Stat {
    comm: String,
    ppid: u32,
    pgid: u32,
    tpgid: u32,
    tty_dev: u64,
    start_ticks: u64,
}

fn parse_stat(text: &str) -> Option<Stat> {
    // comm can contain spaces and ')' characters; fields begin after the LAST closing ')'.
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let fields: Vec<&str> = text.get(close + 1..)?.split_whitespace().collect();
    let encoded = fields.get(4)?.parse::<i32>().ok()? as u32;
    let major = (encoded >> 8) & 0xfff;
    let minor = (encoded & 0xff) | ((encoded >> 12) & 0xfff00);
    Some(Stat {
        comm: text.get(open + 1..close)?.to_string(),
        ppid: fields.get(1)?.parse().ok()?,
        pgid: fields.get(2)?.parse().ok()?,
        tpgid: fields.get(5)?.parse::<i32>().ok().filter(|p| *p > 0).unwrap_or(0) as u32,
        tty_dev: libc::makedev(major, minor),
        start_ticks: fields.get(19)?.parse().ok()?,
    })
}

fn stat(pid: u32) -> Option<Stat> {
    parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// When the process started, in milliseconds: boot time from the clock and `/proc/uptime` (its
/// `btime` is whole seconds) plus the clock ticks it started after boot.
fn start_ms(ticks: u64) -> u64 {
    static BOOT_MS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let boot = *BOOT_MS.get_or_init(|| {
        let up = std::fs::read_to_string("/proc/uptime").ok().and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok());
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        up.map_or(0, |up| now.saturating_sub((up * 1000.0) as u64))
    });
    let rate = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as u64;
    if boot == 0 { 0 } else { boot + ticks * 1000 / rate }
}

fn start_time(ticks: u64) -> u64 {
    static BOOT: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let boot = *BOOT.get_or_init(|| {
        std::fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|s| s.lines().find_map(|l| l.strip_prefix("btime ").and_then(|n| n.parse().ok())))
            .unwrap_or(0)
    });
    let rate = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    boot + ticks / rate.max(1) as u64
}

fn tty_name(pid: u32, dev: u64) -> Option<String> {
    if dev == 0 {
        return None;
    }
    // stdin may be redirected; match the actual controlling device, not just fd 0.
    for fd in 0..3 {
        let path = PathBuf::from(format!("/proc/{pid}/fd/{fd}"));
        if std::fs::metadata(&path).is_ok_and(|m| m.rdev() == dev)
            && let Ok(target) = std::fs::read_link(path)
            && let Ok(name) = target.strip_prefix("/dev")
        {
            return Some(name.to_string_lossy().into_owned());
        }
    }
    let major = libc::major(dev);
    let minor = libc::minor(dev);
    let name = match major {
        136..=143 => format!("pts/{}", (major - 136) * 256 + minor),
        4 => format!("tty{minor}"),
        _ => return None,
    };
    std::fs::metadata(format!("/dev/{name}")).ok().filter(|m| m.rdev() == dev).map(|_| name)
}

pub fn live(pid: u32) -> Option<Live> {
    let info = stat(pid)?;
    Some(Live {
        pgid: info.pgid,
        tpgid: info.tpgid,
        tty: tty_name(pid, info.tty_dev),
        start: start_time(info.start_ticks),
        exe: exe_path(pid),
        comm: info.comm,
    })
}

pub fn exe_path(pid: u32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok().map(|p| p.to_string_lossy().into_owned())
}

pub fn parent_of(pid: u32) -> Option<u32> {
    stat(pid).map(|s| s.ppid)
}

pub fn ancestors_of(pid: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut cur = parent_of(pid);
    while let Some(p) = cur {
        if p <= 1 || out.len() >= 8 || out.contains(&p) {
            break;
        }
        out.push(p);
        cur = parent_of(p);
    }
    out
}

pub(super) fn ps_rows() -> Vec<PsRow> {
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|pid| {
            let info = stat(pid)?;
            Some(PsRow {
                pid,
                ppid: info.ppid,
                tty: tty_name(pid, info.tty_dev),
                comm: info.comm,
                start: start_time(info.start_ticks),
                start_ms: start_ms(info.start_ticks),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_handles_spaces_and_parentheses_in_comm() {
        let s = parse_stat("123 (agent (test)) S 12 123 12 34817 123 0 0 0 0 0 0 0 0 0 0 0 0 0 456 0").unwrap();
        assert_eq!(s.comm, "agent (test)");
        assert_eq!((s.ppid, s.pgid, s.tpgid, s.start_ticks), (12, 123, 123, 456));
        assert_eq!((libc::major(s.tty_dev), libc::minor(s.tty_dev)), (136, 1));
        assert!(parse_stat("not a stat").is_none());
    }

    #[test]
    fn live_identity_matches_process_snapshot() {
        let pid = std::process::id();
        let row = ps_rows().into_iter().find(|r| r.pid == pid).unwrap();
        let live = live(pid).unwrap();
        assert_eq!(row.start, live.start);
        assert_eq!(row.tty, live.tty);
        assert_eq!(parent_of(pid), Some(row.ppid));
        assert_eq!(exe_path(pid).unwrap(), std::env::current_exe().unwrap().to_string_lossy());
        assert!(live.start > 0);
    }
}
