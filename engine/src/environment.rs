//! The Unix launch environment, harvested once and held only in memory. Agent launches and
//! discovery use this same PATH; no automated launch sources the user's interactive rc files.
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(test)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, bail};

type Values = BTreeMap<OsString, OsString>;
const MAX_BYTES: usize = 256 * 1024;
const TIMEOUT: Duration = Duration::from_secs(3);

// Deliberately no Debug/Serialize: this map contains the user's exported credentials.
#[derive(Clone)]
pub struct Environment {
    values: Values,
}

pub fn login() -> &'static Environment {
    static ENV: OnceLock<Environment> = OnceLock::new();
    ENV.get_or_init(|| {
        let base = sanitized(std::env::vars_os().collect(), true);
        // Unit tests exercise real shells only through explicit disposable fixtures below.
        #[cfg(not(test))]
        let harvested = {
            let shell = std::env::var_os("SHELL")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| if cfg!(target_os = "macos") { "/bin/zsh".into() } else { "/bin/bash".into() });
            harvest(Path::new(&shell), &base, TIMEOUT, MAX_BYTES)
        };
        #[cfg(test)]
        let harvested: Result<Values> = Err(anyhow::anyhow!("test process"));
        let mut values = base;
        match harvested {
            Ok(found) => {
                values.extend(found);
                crate::elog::line("shell environment: harvested once");
            }
            Err(_) => crate::elog::line("shell environment: unavailable; using inherited environment and install folders"),
        }
        Environment::new(values, &crate::transcripts::home())
    })
}

fn valid_name(name: &OsStr) -> bool {
    let b = name.as_bytes();
    !b.is_empty() && (b[0].is_ascii_alphabetic() || b[0] == b'_') && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

fn sanitized(values: Values, inherited: bool) -> Values {
    values
        .into_iter()
        .filter(|(key, _)| {
            let name = key.to_string_lossy();
            let upper = name.to_ascii_uppercase();
            valid_name(key)
                && !(inherited && (upper.starts_with("CLAUDE") || upper.starts_with("ANTHROPIC_")))
                && !upper.starts_with("LANTERN_")
                && !upper.starts_with("OCTOBER_BUS_")
                && !upper.starts_with("OCTOBER_SUPABASE_")
                && upper != "OCTOBER_TOKEN_COMMAND"
                && !matches!(
                    name.as_ref(),
                    "BASH_ENV"
                        | "ENV"
                        | "SHELLOPTS"
                        | "BASHOPTS"
                        | "BASH_VERSINFO"
                        | "UID"
                        | "EUID"
                        | "PPID"
                        | "SHLVL"
                        | "_"
                        | "PWD"
                        | "OLDPWD"
                        | "CLAUDE_JOB_DIR"
                )
        })
        .collect()
}

fn harvest(shell: &Path, base: &Values, timeout: Duration, max_bytes: usize) -> Result<Values> {
    match shell.file_name().and_then(OsStr::to_str) {
        Some("zsh" | "bash" | "fish") => {}
        _ => bail!("unsupported login shell"),
    }
    let marker = format!("__LANTERN_ENV_{}__", uuid::Uuid::new_v4().simple());
    // This syntax works in all three supported shell families. stdout is a pipe and stdin EOF;
    // tty-gated updaters do not run. Startup output before the random marker is ignored.
    let script = format!("printf '%s\\0' '{marker}'; /usr/bin/env -0 && printf '%s\\0' '{marker}_END'");
    let out = crate::run::output_capped(Command::new(shell).args(["-ilc", &script]).env_clear().envs(base), timeout, max_bytes)?;
    if !out.status.success() {
        bail!("login environment probe failed");
    }
    parse(&out.stdout, marker.as_bytes()).context("invalid login environment response")
}

fn parse(output: &[u8], marker: &[u8]) -> Option<Values> {
    let marker: Vec<u8> = marker.iter().copied().chain([0]).collect();
    let at = output.windows(marker.len()).position(|w| w == marker)? + marker.len();
    let end: Vec<u8> = marker[..marker.len() - 1].iter().copied().chain(b"_END\0".iter().copied()).collect();
    let records = output[at..].strip_suffix(end.as_slice())?;
    let mut values = Values::new();
    if records.is_empty() {
        return Some(values);
    }
    if !records.ends_with(&[0]) {
        return None;
    }
    for record in records[..records.len() - 1].split(|b| *b == 0) {
        let equals = record.iter().position(|b| *b == b'=')?;
        let name = OsString::from_vec(record[..equals].to_vec());
        // Bash exports functions as BASH_FUNC_name%% records. They are deliberately not
        // launchable installations; skip their definitions without losing real exports.
        if name.as_bytes().starts_with(b"BASH_FUNC_") && name.as_bytes().ends_with(b"%%") {
            continue;
        }
        if !valid_name(&name) || values.contains_key(&name) {
            return None;
        }
        values.insert(name, OsString::from_vec(record[equals + 1..].to_vec()));
    }
    Some(values)
}

impl Environment {
    pub(crate) fn new(values: Values, home: &Path) -> Self {
        let mut this = Self { values: sanitized(values, false) };
        this.values.insert("TERM".into(), "xterm-256color".into());
        this.values.insert("TERM_PROGRAM".into(), "October Lantern".into());
        this.values.entry("LANG".into()).or_insert_with(|| if cfg!(target_os = "macos") { "en_US.UTF-8".into() } else { "C.UTF-8".into() });
        let mut dirs: Vec<PathBuf> = this.values.get(OsStr::new("PATH")).map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
        dirs.extend(install_folders(home));
        let mut seen = std::collections::HashSet::new();
        dirs.retain(|p| {
            p.is_absolute()
                && !p.as_os_str().as_bytes().contains(&b':')
                && !p.ends_with("node_modules/.bin")
                && !p.ends_with("node-gyp-bin")
                && seen.insert(p.clone())
        });
        this.values.insert("PATH".into(), std::env::join_paths(dirs).expect("existing PATH components have no separators"));
        this
    }

    pub fn overlay(&mut self, values: &[(String, String)]) -> Result<()> {
        for (key, value) in values {
            if !valid_name(OsStr::new(key)) || value.as_bytes().contains(&0) {
                bail!("invalid launch environment entry");
            }
            self.values.insert(key.into(), value.into());
        }
        Ok(())
    }

    pub fn prepend(&mut self, dir: &Path) -> Result<()> {
        if !dir.is_absolute() {
            bail!("launch runtime directory must be absolute");
        }
        let dirs = std::iter::once(dir.to_owned()).chain(std::env::split_paths(self.values.get(OsStr::new("PATH")).unwrap()));
        self.values.insert("PATH".into(), std::env::join_paths(dirs).context("invalid launch runtime path")?);
        Ok(())
    }

    pub fn find(&self, name: &str) -> Option<PathBuf> {
        std::env::split_paths(self.values.get(OsStr::new("PATH"))?).map(|p| p.join(name)).find(|p| executable(p))
    }

    pub fn get(&self, name: &str) -> Option<&OsStr> {
        self.values.get(OsStr::new(name)).map(OsString::as_os_str)
    }

    pub fn command(&self, name: &str) -> Result<Command> {
        let path = self.find(name).context("program is not installed on the launch PATH")?;
        let mut cmd = Command::new(path);
        cmd.env_clear().envs(&self.values);
        Ok(cmd)
    }

    pub fn exports(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (key, value) in &self.values {
            out.extend_from_slice(b"export ");
            out.extend_from_slice(key.as_bytes());
            out.extend_from_slice(b"='");
            for b in value.as_bytes() {
                if *b == b'\'' {
                    out.extend_from_slice(b"'\\''");
                } else {
                    out.push(*b);
                }
            }
            out.extend_from_slice(b"'\n");
        }
        out
    }
}

pub(crate) fn executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file())
        && std::ffi::CString::new(path.as_os_str().as_bytes()).is_ok_and(|p| {
            // SAFETY: p is a valid NUL-terminated path; access only inspects this user's access.
            unsafe { libc::access(p.as_ptr(), libc::X_OK) == 0 }
        })
}

fn install_folders(home: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = [
        ".local/bin",
        ".claude/local",
        ".npm-global/bin",
        ".bun/bin",
        ".volta/bin",
        ".cargo/bin",
        ".opencode/bin",
        ".asdf/shims",
        ".local/share/mise/shims",
        ".nix-profile/bin",
    ]
    .into_iter()
    .map(|p| home.join(p))
    .collect();
    dirs.extend(
        ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"].into_iter().map(PathBuf::from),
    );
    for root in [".nvm/versions/node", ".local/share/fnm/node-versions", "Library/Application Support/fnm/node-versions"] {
        let mut versions: Vec<_> = std::fs::read_dir(home.join(root)).into_iter().flatten().flatten().map(|e| e.path()).collect();
        versions.sort();
        for version in versions {
            dirs.push(version.join("bin"));
            dirs.push(version.join("installation/bin"));
        }
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("lantern-env-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn write(&self, name: &str, content: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, content).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            path
        }
        fn base(&self) -> Values {
            [("HOME", self.0.as_os_str()), ("ZDOTDIR", self.0.as_os_str()), ("PATH", OsStr::new("/usr/bin:/bin"))]
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn nul_protocol_preserves_values_and_refuses_partial_or_malformed_results() {
        let bytes = b"startup banner\nkey=value\nMARK\0PATH=/x:/bin\0TOKEN=a=b\n'quoted'\0BINARY=\xff\0MARK_END\0";
        let parsed = parse(bytes, b"MARK").unwrap();
        assert!(parse(b"MARK\0BASH_FUNC_demo%%=() { true; }\0PATH=/bin\0MARK_END\0", b"MARK").unwrap().contains_key(OsStr::new("PATH")));
        assert_eq!(parsed[OsStr::new("TOKEN")], OsStr::new("a=b\n'quoted'"));
        assert_eq!(parsed[OsStr::new("BINARY")].as_bytes(), b"\xff");
        for bad in
            [&b"MARK\0PATH=/x"[..], &b"MARK\0bad-key=value\0MARK_END\0"[..], &b"MARK\0PATH=/x\0PATH=/y\0MARK_END\0"[..], &b"PATH=/x\0"[..]]
        {
            assert!(parse(bad, b"MARK").is_none());
        }
    }

    #[test]
    fn real_login_shells_run_once_without_a_tty_and_reads_get_eof() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.0.join("bin")).unwrap();
        fixture.write("bin/lantern-env-fixture", "#!/bin/sh\nprintf '%s' \"$ANTHROPIC_API_KEY|$LOGIN_ONLY|$PATH\" > \"$HOME/receipt\"\n");
        let rc = "printf x >> \"$HOME/passes\"\nexport ANTHROPIC_API_KEY='user-fixture-key'\nexport PATH=\"$HOME/bin:$PATH\"\nif ( : </dev/tty ) 2>/dev/null; then export BAD_TTY=1; fi\nif [ -t 1 ]; then printf 'Would you like to update? [Y/n]'; while :; do read answer; done; fi\nread answer || true\n";
        fixture.write(".zprofile", "export LOGIN_ONLY=from-login\n");
        fixture.write(".zshrc", rc);
        fixture.write(".bash_profile", &format!("export LOGIN_ONLY=from-login\n{rc}"));
        std::fs::create_dir_all(fixture.0.join(".config/fish")).unwrap();
        fixture.write(".config/fish/config.fish", "printf x >> \"$HOME/passes\"\nset -gx ANTHROPIC_API_KEY user-fixture-key\nset -gx LOGIN_ONLY from-login\nset -gx PATH \"$HOME/bin\" $PATH\nread answer; or true\n");
        for shell in ["/bin/zsh", "/bin/bash", "/usr/bin/fish", "/opt/homebrew/bin/fish", "/usr/local/bin/fish"]
            .into_iter()
            .filter(|s| Path::new(s).exists())
        {
            let _ = std::fs::remove_file(fixture.0.join("passes"));
            let mut base = fixture.base();
            base.insert("XDG_CONFIG_HOME".into(), fixture.0.join(".config").into_os_string());
            base.insert("ANTHROPIC_API_KEY".into(), "parent-key-must-not-survive".into());
            let clean = sanitized(base, true);
            assert!(!clean.contains_key(OsStr::new("ANTHROPIC_API_KEY")));
            let found = harvest(Path::new(shell), &clean, TIMEOUT, MAX_BYTES).unwrap();
            assert!(!found.contains_key(OsStr::new("BAD_TTY")), "harvest inherited a controlling terminal");
            let mut merged = clean;
            merged.extend(found);
            let env = Environment::new(merged, &fixture.0);
            for _ in 0..2 {
                let out = crate::run::output(&mut env.command("lantern-env-fixture").unwrap(), TIMEOUT).unwrap();
                assert!(out.status.success());
            }
            assert_eq!(std::fs::read_to_string(fixture.0.join("passes")).unwrap(), "x");
            let receipt = std::fs::read_to_string(fixture.0.join("receipt")).unwrap();
            assert!(receipt.starts_with("user-fixture-key|from-login|"), "{shell}");
            assert!(receipt.contains(&fixture.0.join("bin").to_string_lossy().to_string()));
        }
    }

    fn released(pid: i32) -> bool {
        let out = Command::new("/bin/ps").args(["-p", &pid.to_string(), "-o", "stat="]).output().unwrap();
        !out.status.success() || String::from_utf8_lossy(&out.stdout).trim().starts_with('Z') || out.stdout.is_empty()
    }

    #[test]
    fn deadline_output_cap_and_success_cleanup_stop_owned_descendants() {
        for (body, timeout, cap, expected_ok) in [
            ("/bin/sleep 20 & echo $! > \"$HOME/child\"; wait", Duration::from_millis(180), MAX_BYTES, false),
            ("/bin/sleep 20 & echo $! > \"$HOME/child\"; /bin/sh -c \"$2\"; exit 0", Duration::from_millis(180), MAX_BYTES, false),
            ("/bin/sleep 20 >/dev/null 2>&1 & echo $! > \"$HOME/child\"; exec /bin/sh -c \"$2\"", TIMEOUT, MAX_BYTES, true),
            ("while :; do printf 'dummy-credential-never-log'; done", Duration::from_secs(2), 512, false),
        ] {
            let fixture = Fixture::new();
            let shell = fixture.write("bash", &format!("#!/bin/sh\n{body}\n"));
            let started = std::time::Instant::now();
            let result = harvest(&shell, &fixture.base(), timeout, cap);
            assert_eq!(result.is_ok(), expected_ok, "{}", result.as_ref().err().map(|e| format!("{e:#}")).unwrap_or_default());
            assert!(started.elapsed() < Duration::from_secs(4));
            if let Err(e) = result {
                assert!(!format!("{e:#}").contains("dummy-credential"));
            }
            if let Ok(pid) = std::fs::read_to_string(fixture.0.join("child")) {
                let pid = pid.trim().parse().unwrap();
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while !released(pid) {
                    assert!(std::time::Instant::now() < deadline, "harvest descendant still running");
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    #[test]
    fn fallback_resolution_and_private_overrides_use_the_childs_actual_path() {
        let fixture = Fixture::new();
        std::fs::create_dir_all(fixture.0.join(".local/bin")).unwrap();
        let binary = fixture.write(".local/bin/lantern-env-fixture", "#!/bin/sh\n/usr/bin/env -0\n");
        fixture.write(".local/bin/not-executable", "no");
        std::fs::set_permissions(fixture.0.join(".local/bin/not-executable"), std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut env = Environment::new(fixture.base(), &fixture.0);
        assert_eq!(env.find("lantern-env-fixture"), Some(binary));
        assert!(env.find("not-executable").is_none());
        env.overlay(&[("ANTHROPIC_API_KEY".into(), "explicit-user-key".into()), ("LANTERN_PRIVATE_ID".into(), "correct-launch".into())])
            .unwrap();
        env.prepend(&fixture.0.join(".local/bin")).unwrap();
        let mut cmd = env.command("lantern-env-fixture").unwrap();
        let out = crate::run::output(&mut cmd, TIMEOUT).unwrap();
        let mut marked = b"ENV\0".to_vec();
        marked.extend(out.stdout);
        marked.extend(b"ENV_END\0");
        let values = parse(&marked, b"ENV").unwrap();
        assert_eq!(values[OsStr::new("PATH")], env.values[OsStr::new("PATH")]);
        assert_eq!(values[OsStr::new("LANTERN_PRIVATE_ID")], "correct-launch");
        assert_eq!(values[OsStr::new("ANTHROPIC_API_KEY")], "explicit-user-key");
        assert!(env.overlay(&[("INVALID-NAME".into(), "value".into())]).is_err());
        assert!(env.overlay(&[("VALID".into(), "contains\0nul".into())]).is_err());
    }

    #[test]
    fn unknown_shell_is_not_executed_and_parent_private_state_is_not_inherited() {
        let fixture = Fixture::new();
        let tty_probe = fixture.write("bash", "#!/bin/sh\nif ( : </dev/tty ) 2>/dev/null; then exit 9; fi\nexec /bin/sh -c \"$2\"\n");
        assert!(harvest(&tty_probe, &fixture.base(), TIMEOUT, MAX_BYTES).is_ok(), "harvest retained a controlling terminal");
        let shell = fixture.write("unknown-shell", "#!/bin/sh\nexit 0\n");
        assert!(harvest(&shell, &fixture.base(), TIMEOUT, MAX_BYTES).is_err());
        let map = [
            ("CLAUDECODE", "1"),
            ("ANTHROPIC_AUTH_TOKEN", "parent"),
            ("LANTERN_CARD_KEY", "parent"),
            ("OCTOBER_BUS_AGENT_TOKEN", "parent"),
            ("OCTOBER_SUPABASE_REFRESH_TOKEN", "parent"),
            ("BASH_ENV", "/tmp/no"),
            ("GH_TOKEN", "user-token"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let clean = sanitized(map, true);
        assert_eq!(clean.len(), 1);
        assert_eq!(clean[OsStr::new("GH_TOKEN")], "user-token");
    }
}
