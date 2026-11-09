use super::*;

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

fn bsd_info(pid: i32) -> Option<libc::proc_bsdinfo> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let got = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
    (got == size).then_some(info)
}

/// Parent, terminal and name for every process, straight from the kernel (what `ps` reads, without
/// the cost of launching `ps` on every scan). Works for root-owned processes too.
pub(super) fn ps_rows() -> Vec<PsRow> {
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
            Some(PsRow {
                pid: pid as u32,
                ppid: info.pbi_ppid,
                tty: tty_name(info.e_tdev),
                comm,
                start: info.pbi_start_tvsec,
                start_ms: info.pbi_start_tvsec * 1000 + info.pbi_start_tvusec / 1000,
            })
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
    (got == size).then(|| PsRow { pid: pid as u32, ppid: info.ppid, tty: None, comm: c_str(&info.comm), start: 0, start_ms: 0 })
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
