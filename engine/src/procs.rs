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
                entry.cwd = p.cwd().map(PathBuf::from);
            }
        }
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

/// `None` when the process is gone (or belongs to another user, which agents never do).
pub fn live(pid: u32) -> Option<Live> {
    let info = bsd_info(pid as i32)?;
    Some(Live {
        pgid: info.pbi_pgid,
        tpgid: info.e_tpgid,
        tty: tty_name(info.e_tdev),
        start: info.pbi_start_tvsec,
        exe: exe_path(pid),
        comm: c_str(&info.pbi_comm),
    })
}

/// The executable a process is running right now, from the kernel.
pub fn exe_path(pid: u32) -> Option<String> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let n = unsafe { libc::proc_pidpath(pid as i32, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    String::from_utf8(buf).ok()
}

/// Parent pid of `pid`, including root-owned processes; `None` when it's gone.
pub fn parent_of(pid: u32) -> Option<u32> {
    bsd_info(pid as i32).map(|i| i.pbi_ppid).or_else(|| short_info(pid as i32).map(|r| r.ppid))
}

/// Process ids above `pid`, nearest first, stopping at launchd.
pub fn ancestors_of(pid: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut cur = parent_of(pid);
    while let Some(p) = cur {
        if p <= 1 || out.len() >= 8 {
            break;
        }
        out.push(p);
        cur = parent_of(p);
    }
    out
}

struct PsRow {
    pid: u32,
    ppid: u32,
    tty: Option<String>,
    comm: String,
    /// Seconds since the epoch; 0 when unknown.
    start: u64,
}

fn bsd_info(pid: i32) -> Option<libc::proc_bsdinfo> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let got = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
    (got == size).then_some(info)
}

/// Parent, terminal and name for every process, straight from the kernel (what `ps` reads, without
/// the cost of launching `ps` on every scan). Works for root-owned processes too.
fn ps_rows() -> Vec<PsRow> {
    let mut pids = vec![0i32; 8192];
    let bytes = (pids.len() * std::mem::size_of::<i32>()) as i32;
    let n = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    if n <= 0 {
        return Vec::new();
    }
    pids.truncate(n as usize);
    pids.into_iter()
        .filter(|p| *p > 0)
        .filter_map(|pid| {
            let Some(info) = bsd_info(pid) else {
                // Root-owned processes (e.g. /usr/bin/login between a terminal app and its shell)
                // only answer the short query; that's enough to keep the parent chain intact.
                return short_info(pid);
            };
            let name = c_str(&info.pbi_name);
            let comm = if name.is_empty() { c_str(&info.pbi_comm) } else { name };
            Some(PsRow { pid: pid as u32, ppid: info.pbi_ppid, tty: tty_name(info.e_tdev), comm, start: info.pbi_start_tvsec })
        })
        .collect()
}

/// `struct proc_bsdshortinfo` from <sys/proc_info.h> (not in the libc crate).
#[repr(C)]
struct ProcBsdShortInfo {
    pid: u32,
    ppid: u32,
    pgid: u32,
    status: u32,
    comm: [libc::c_char; 16],
    flags: u32,
    uid: u32,
    gid: u32,
    ruid: u32,
    rgid: u32,
    svuid: u32,
    svgid: u32,
    rfu: u32,
}

const PROC_PIDT_SHORTBSDINFO: i32 = 13;

fn short_info(pid: i32) -> Option<PsRow> {
    let mut info: ProcBsdShortInfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<ProcBsdShortInfo>() as i32;
    let got = unsafe { libc::proc_pidinfo(pid, PROC_PIDT_SHORTBSDINFO, 0, (&mut info as *mut ProcBsdShortInfo).cast(), size) };
    (got == size).then(|| PsRow { pid: pid as u32, ppid: info.ppid, tty: None, comm: c_str(&info.comm), start: 0 })
}

fn c_str(chars: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = chars.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `ttys004` for a controlling terminal device, `None` without one. `devname` searches /dev, so
/// names are remembered per device number.
fn tty_name(dev: u32) -> Option<String> {
    use std::sync::Mutex;
    static NAMES: Mutex<Option<HashMap<u32, Option<String>>>> = Mutex::new(None);
    if dev == 0 || dev == u32::MAX {
        return None;
    }
    let mut names = NAMES.lock().unwrap();
    names.get_or_insert_with(HashMap::new).entry(dev).or_insert_with(|| lookup_tty(dev)).clone()
}

fn lookup_tty(dev: u32) -> Option<String> {
    let name = unsafe { libc::devname(dev as libc::dev_t, libc::S_IFCHR) };
    if name.is_null() {
        return None;
    }
    let s = unsafe { std::ffi::CStr::from_ptr(name) }.to_string_lossy().into_owned();
    if s.is_empty() || s == "??" { None } else { Some(s) }
}

/// The last path component, for argv entries and executable paths.
pub fn basename(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}
