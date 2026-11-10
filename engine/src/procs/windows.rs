//! Windows process identity from the Toolhelp snapshot and per-process queries.
//!
//! Windows has no controlling tty. What stands in for one is the process's console host: each
//! console (a conhost window, or a Windows Terminal / VS Code tab through ConPTY) is served by its
//! own `conhost.exe` or `OpenConsole.exe`, so `con:<host pid>` names one terminal the way
//! `pts/3` does. Nothing here knows a console's foreground process, so `Live` reports it as
//! unknown and delivery refuses to type (see `deliver::check`).
use super::*;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

use windows_sys::Wdk::System::Threading::NtQueryInformationProcess;
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

/// `PROCESSINFOCLASS::ProcessConsoleHostProcess`: the pid of the process's console host. The low
/// two bits are flags.
const PROCESS_CONSOLE_HOST_PROCESS: i32 = 49;

/// Seconds between 1601-01-01 (FILETIME's epoch) and 1970-01-01.
const EPOCH_DIFF_SECS: u64 = 11_644_473_600;

struct Handle(HANDLE);

impl Handle {
    fn open(pid: u32) -> Option<Handle> {
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        (!h.is_null()).then_some(Handle(h))
    }

    /// Seconds since the epoch; 0 when unknown.
    fn start(&self) -> u64 {
        self.start_ms() / 1000
    }

    /// Milliseconds since the epoch; 0 when unknown.
    fn start_ms(&self) -> u64 {
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        if unsafe { GetProcessTimes(self.0, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
            return 0;
        }
        let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        (ticks / 10_000).saturating_sub(EPOCH_DIFF_SECS * 1000)
    }

    fn exe(&self) -> Option<String> {
        let mut buf = vec![0u16; 32 * 1024];
        let mut len = buf.len() as u32;
        if unsafe { QueryFullProcessImageNameW(self.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) } == 0 {
            return None;
        }
        Some(OsString::from_wide(&buf[..len as usize]).to_string_lossy().into_owned())
    }

    /// `con:<pid of its console host>`, or `None` for a process without a console.
    fn console(&self) -> Option<String> {
        let mut host: usize = 0;
        let status = unsafe {
            NtQueryInformationProcess(
                self.0,
                PROCESS_CONSOLE_HOST_PROCESS,
                (&mut host as *mut usize).cast(),
                std::mem::size_of::<usize>() as u32,
                std::ptr::null_mut(),
            )
        };
        let host = host & !3;
        (status >= 0 && host != 0).then(|| format!("con:{host}"))
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Every process: pid, parent pid and executable file name (e.g. `claude.exe`).
fn snapshot() -> Vec<(u32, u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut entry) != 0;
        while ok {
            let name = &entry.szExeFile;
            let len = name.iter().position(|c| *c == 0).unwrap_or(name.len());
            out.push((entry.th32ProcessID, entry.th32ParentProcessID, String::from_utf16_lossy(&name[..len])));
            ok = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
    }
    out
}

/// Windows keeps a process's parent pid after the parent exits, and pids are reused, so a parent
/// that started after its child is some other process: treat it as no parent (0).
fn real_parent(ppid: u32, start: u64, starts: &HashMap<u32, u64>) -> u32 {
    match starts.get(&ppid) {
        Some(&parent) if parent != 0 && start != 0 && parent > start => 0,
        Some(_) => ppid,
        None => 0,
    }
}

pub fn live(pid: u32) -> Option<Live> {
    let h = Handle::open(pid)?;
    let exe = h.exe();
    let comm = exe.as_deref().map(|e| e.rsplit('\\').next().unwrap_or(e).to_string()).unwrap_or_default();
    Some(Live { pgid: 0, tpgid: 0, tty: h.console(), start: h.start(), exe, comm })
}

pub fn ancestors_of(pid: u32) -> Vec<u32> {
    let rows = snapshot();
    let starts: HashMap<u32, u64> = rows.iter().map(|(p, _, _)| (*p, Handle::open(*p).map_or(0, |h| h.start()))).collect();
    let parents: HashMap<u32, u32> = rows.iter().map(|(p, pp, _)| (*p, real_parent(*pp, starts[p], &starts))).collect();
    let mut out = Vec::new();
    let mut cur = parents.get(&pid).copied();
    while let Some(p) = cur {
        if p <= 4 || out.len() >= 8 || out.contains(&p) {
            break;
        }
        out.push(p);
        cur = parents.get(&p).copied();
    }
    out
}

pub(super) fn ps_rows() -> Vec<PsRow> {
    let rows: Vec<(u32, u32, String, u64, Option<String>)> = snapshot()
        .into_iter()
        .map(|(pid, ppid, comm)| {
            let (start_ms, tty) = Handle::open(pid).map_or((0, None), |h| (h.start_ms(), h.console()));
            (pid, ppid, comm, start_ms, tty)
        })
        .collect();
    let starts: HashMap<u32, u64> = rows.iter().map(|r| (r.0, r.3 / 1000)).collect();
    rows.into_iter()
        .map(|(pid, ppid, comm, start_ms, tty)| {
            let start = start_ms / 1000;
            PsRow { pid, ppid: real_parent(ppid, start, &starts), tty, comm, start, start_ms }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reused_parent_pid_is_not_a_parent() {
        let starts = HashMap::from([(10, 100), (20, 300), (30, 0)]);
        assert_eq!(real_parent(10, 200, &starts), 10);
        assert_eq!(real_parent(20, 200, &starts), 0);
        assert_eq!(real_parent(30, 200, &starts), 30);
        assert_eq!(real_parent(99, 200, &starts), 0);
    }

    #[test]
    fn live_identity_matches_process_snapshot() {
        let pid = std::process::id();
        let row = ps_rows().into_iter().find(|r| r.pid == pid).unwrap();
        let live = live(pid).unwrap();
        assert_eq!(row.start, live.start);
        assert_eq!(row.tty, live.tty);
        assert!(live.start > 0);
        assert_eq!(live.exe.unwrap(), std::env::current_exe().unwrap().to_string_lossy());
        assert!(ancestors_of(pid).contains(&row.ppid) || row.ppid == 0);
    }
}
