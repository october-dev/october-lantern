//! A snapshot of the process table: argv, cwd, parent and controlling tty for every process.

use std::collections::HashMap;
use std::path::PathBuf;

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

/// Command lines and executable paths, remembered per process so each process is only asked once.
/// Keyed by pid, start time and the kernel's name for it: a reused pid, or a process that `exec`ed
/// another program (same pid and start time, new name), is asked again.
#[derive(Default)]
pub struct ProcCache {
    entries: HashMap<u32, Cached>,
    captures: u32,
}

struct Cached {
    start: u64,
    /// The kernel's name when asked; a different one means the process ran another program.
    comm: String,
    name: String,
    exe: Option<PathBuf>,
    cmd: Vec<String>,
}

impl ProcTable {
    pub fn capture(sys: &mut System, cache: &mut ProcCache) -> ProcTable {
        // The kernel's process list is the source of truth for the tree; sysinfo can't read some
        // root-owned processes (e.g. `/usr/bin/login` between a terminal app and the shell).
        let rows = ps_rows();

        let new: Vec<sysinfo::Pid> = rows
            .iter()
            .filter(|r| cache.entries.get(&r.pid).is_none_or(|e| e.start != r.start || e.comm != r.comm))
            .map(|r| sysinfo::Pid::from_u32(r.pid))
            .collect();
        if !new.is_empty() {
            sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&new),
                true,
                ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always).with_exe(UpdateKind::Always),
            );
            for pid in &new {
                let row = rows.iter().find(|r| r.pid == pid.as_u32());
                let (start, comm) = row.map(|r| (r.start, r.comm.clone())).unwrap_or_default();
                let (name, exe, cmd) = match sys.process(*pid) {
                    Some(p) => (
                        p.name().to_string_lossy().into_owned(),
                        p.exe().map(PathBuf::from),
                        p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect(),
                    ),
                    None => (String::new(), None, Vec::new()),
                };
                cache.entries.insert(pid.as_u32(), Cached { start, comm, name, exe, cmd });
            }
        }
        let live: std::collections::HashSet<u32> = rows.iter().map(|r| r.pid).collect();
        cache.entries.retain(|pid, _| live.contains(pid));
        // sysinfo keeps every process it has seen; start over now and then so it doesn't grow.
        cache.captures += 1;
        if cache.captures.is_multiple_of(400) {
            *sys = System::new();
        }

        let procs = rows
            .into_iter()
            .map(|row| {
                let cached = cache.entries.get(&row.pid);
                let name = cached.map(|c| c.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| basename(&row.comm).to_string());
                let exe = cached.and_then(|c| c.exe.clone()).or_else(|| Some(PathBuf::from(&row.comm)));
                let proc = Proc {
                    pid: row.pid,
                    ppid: Some(row.ppid),
                    name,
                    exe,
                    cmd: cached.map(|c| c.cmd.clone()).unwrap_or_default(),
                    cwd: None,
                    start_time: row.start,
                    tty: row.tty,
                };
                (row.pid, proc)
            })
            .collect();
        ProcTable { procs }
    }

    /// The first child of `pid`, e.g. the native codex binary under its node wrapper.
    pub fn child_of(&self, pid: u32) -> Option<u32> {
        self.procs.values().filter(|p| p.ppid == Some(pid)).map(|p| p.pid).min()
    }

    /// Fetches working directories (and environments, for cmux ids) for just these processes.
    pub fn fill_cwds(&mut self, sys: &mut System, pids: &[u32]) {
        let list: Vec<sysinfo::Pid> = pids.iter().map(|p| sysinfo::Pid::from_u32(*p)).collect();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&list),
            false,
            ProcessRefreshKind::nothing().with_cwd(UpdateKind::Always).with_environ(UpdateKind::OnlyIfNotSet),
        );
        for pid in pids {
            if let (Some(entry), Some(p)) = (self.procs.get_mut(pid), sys.process(sysinfo::Pid::from_u32(*pid))) {
                entry.cwd = p.cwd().map(clean_cwd);
            }
        }
    }

    /// The October Bus id Lantern gave the agent at `pid`: from its Bus connection under it
    /// (`october-bus mcp stdio --agent <id>`, a child or, under a wrapper, a grandchild), or the
    /// `october-bus agent run --id <id>` it runs under (the October harness).
    pub fn bus_id(&self, pid: u32) -> Option<String> {
        let is_bus = |p: &Proc| p.cmd.first().is_some_and(|c| basename(c).starts_with("october-bus"));
        let flag =
            |p: &Proc, name: &str| p.cmd.windows(2).find(|w| w[0] == name).map(|w| w[1].clone()).filter(|id| id.starts_with("lantern-"));
        let children = |of: u32| self.procs.values().filter(move |p| p.ppid == Some(of));
        let below = children(pid).chain(children(pid).flat_map(|c| children(c.pid)));
        below
            .filter(|p| is_bus(p))
            .find_map(|p| flag(p, "--agent"))
            .or_else(|| self.ancestors(pid).into_iter().take(3).filter(|p| is_bus(p)).find_map(|p| flag(p, "--id")))
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

/// What the kernel says about one live process right now.
pub struct Live {
    /// Its process group, and the foreground process group of its terminal (0 when unknown).
    pub pgid: u32,
    pub tpgid: u32,
    pub tty: Option<String>,
    /// Seconds since the epoch.
    pub start: u64,
    /// The program it runs now, and the kernel's short name for it (both change when the process
    /// `exec`s another program).
    pub exe: Option<String>,
    pub comm: String,
}

struct PsRow {
    pid: u32,
    ppid: u32,
    tty: Option<String>,
    comm: String,
    /// Seconds since the epoch; 0 when unknown.
    start: u64,
}

#[cfg(target_os = "macos")]
#[path = "procs/macos.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "procs/linux.rs"]
mod platform;
#[cfg(windows)]
#[path = "procs/windows.rs"]
mod platform;

use platform::ps_rows;
pub use platform::{ancestors_of, live};

/// Windows keeps a process's working directory with a trailing `\` (`C:\work\app\`); agents
/// name their sessions after it without one, so drop it (but keep a drive's own, `C:\`).
fn clean_cwd(path: &std::path::Path) -> PathBuf {
    if cfg!(windows) {
        let text = path.to_string_lossy();
        let trimmed = text.trim_end_matches(['\\', '/']);
        if trimmed.len() < text.len() && !trimmed.is_empty() && !trimmed.ends_with(':') {
            return PathBuf::from(trimmed);
        }
    }
    path.to_path_buf()
}

/// The last path component, for argv entries and executable paths. On Windows it also drops the
/// `.exe` extension, so `C:\…\claude.exe` is `claude` as on other systems.
pub fn basename(s: &str) -> &str {
    if cfg!(windows) {
        let name = s.rsplit(['/', '\\']).next().unwrap_or(s);
        return match name.len().checked_sub(4) {
            Some(i) if name.is_char_boundary(i) && name[i..].eq_ignore_ascii_case(".exe") => &name[..i],
            _ => name,
        };
    }
    s.rsplit('/').next().unwrap_or(s)
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn working_directories_lose_the_trailing_separator() {
        assert_eq!(clean_cwd(std::path::Path::new(r"C:\work\app\")), PathBuf::from(r"C:\work\app"));
        assert_eq!(clean_cwd(std::path::Path::new(r"C:\work\app")), PathBuf::from(r"C:\work\app"));
        assert_eq!(clean_cwd(std::path::Path::new(r"C:\")), PathBuf::from(r"C:\"));
    }

    #[test]
    fn program_names_drop_the_folder_and_exe() {
        assert_eq!(basename(r"C:\Users\x\.local\bin\claude.exe"), "claude");
        assert_eq!(basename("CODEX.EXE"), "CODEX");
        assert_eq!(basename("node"), "node");
    }
}
