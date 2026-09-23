//! A snapshot of the process table: argv, cwd, parent and controlling tty for every process.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Debug, Clone)]
pub struct Proc {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub name: String,
    pub exe: Option<PathBuf>,
    pub cmd: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Seconds since the epoch.
    pub start_time: u64,
    /// e.g. `ttys004`; `None` when the process has no controlling terminal.
    pub tty: Option<String>,
}

pub struct ProcTable {
    pub procs: HashMap<u32, Proc>,
}

impl ProcTable {
    pub fn capture(sys: &mut System) -> ProcTable {
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cwd(UpdateKind::Always),
        );
        // `ps` is the source of truth for the tree: sysinfo can't read some processes owned by
        // root (e.g. `/usr/bin/login` between a terminal app and the shell), which would break
        // ancestor walks. sysinfo adds argv, cwd and start time where it can.
        let mut procs: HashMap<u32, Proc> = ps_rows()
            .into_iter()
            .map(|row| {
                let exe = PathBuf::from(&row.comm);
                (
                    row.pid,
                    Proc {
                        pid: row.pid,
                        ppid: Some(row.ppid),
                        name: basename(&row.comm).to_string(),
                        exe: Some(exe),
                        cmd: Vec::new(),
                        cwd: None,
                        start_time: 0,
                        tty: row.tty,
                    },
                )
            })
            .collect();
        for (pid, p) in sys.processes() {
            let Some(entry) = procs.get_mut(&pid.as_u32()) else { continue };
            entry.name = p.name().to_string_lossy().into_owned();
            if let Some(exe) = p.exe() {
                entry.exe = Some(PathBuf::from(exe));
            }
            entry.cmd = p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect();
            entry.cwd = p.cwd().map(PathBuf::from);
            entry.start_time = p.start_time();
        }
        ProcTable { procs }
    }

    pub fn get(&self, pid: u32) -> Option<&Proc> {
        self.procs.get(&pid)
    }

    /// Parents of `pid`, nearest first. Stops at launchd and guards against cycles.
    pub fn ancestors(&self, pid: u32) -> Vec<&Proc> {
        let mut out = Vec::new();
        let mut cur = self.get(pid).and_then(|p| p.ppid);
        while let Some(p) = cur {
            if p <= 1 || out.len() > 64 {
                break;
            }
            match self.get(p) {
                Some(proc) => {
                    out.push(proc);
                    cur = proc.ppid;
                }
                None => break,
            }
        }
        out
    }
}

struct PsRow {
    pid: u32,
    ppid: u32,
    tty: Option<String>,
    comm: String,
}

/// One `ps` call per scan. `comm` goes last so it isn't truncated.
fn ps_rows() -> Vec<PsRow> {
    let Ok(out) = Command::new("/bin/ps").args(["-axo", "pid=,ppid=,tty=,comm="]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse().ok()?;
            let ppid = parts.next()?.parse().ok()?;
            let tty = parts.next()?;
            let comm = parts.collect::<Vec<_>>().join(" ");
            let tty = match tty {
                "??" | "-" => None,
                t if t.starts_with("tty") => Some(t.to_string()),
                t => Some(format!("tty{t}")),
            };
            Some(PsRow { pid, ppid, tty, comm })
        })
        .collect()
}

/// The last path component, for argv entries and executable paths.
pub fn basename(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}
