#!/usr/bin/env python3
"""Check the winget manifests in windows/winget/<version>/ against what the repo builds.

Every folder is checked against its own version: it must point at that version's release
installer and agree with windows/installer.iss (per-user, Inno Setup, same AppId) and NOTICE. A
manifest can only be written once its release is published (it needs the installer's hash), so the
engine's version may be ahead of the newest folder; that's reported, not a failure.

Then the checker checks itself: it breaks a copy of the newest manifest in several ways and
requires each break to be caught.

  python scripts/check-winget-manifest.py            # offline checks, plus `winget validate` if installed
  python scripts/check-winget-manifest.py --online   # also compare InstallerSha256 with the release's .sha256
"""
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
WINGET = ROOT / "windows" / "winget"
PACKAGE = "October.Lantern"
RELEASES = "https://github.com/harshsaver/october-lantern-releases/releases/download"
INSTALLER_NAME = "October-Lantern-windows-x86_64-setup.exe"
FILES = (f"{PACKAGE}.yaml", f"{PACKAGE}.installer.yaml", f"{PACKAGE}.locale.en-US.yaml")


def scalars(path):
    """Top-level `Key: value` lines of a manifest file (the manifests are flat enough for this)."""
    out = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        m = re.match(r"^([A-Za-z]+):\s*(.*?)\s*$", line)
        if m:
            out[m.group(1)] = m.group(2).strip("'\"")
    return out


def check_folder(folder, online=False, validate=True, say=print):
    """Returns the list of failed checks for one windows/winget/<version> folder."""
    failures = []

    def check(ok, message):
        say(("ok    " if ok else "FAIL  ") + message)
        if not ok:
            failures.append(message)

    version = folder.name
    app_id = re.search(r"^AppId=\{(\{[0-9A-Fa-f-]+\})", (ROOT / "windows" / "installer.iss").read_text(), re.M).group(1)
    copyright_line = re.search(r"^(Copyright .+)$", (ROOT / "NOTICE").read_text(), re.M).group(1)

    check(re.fullmatch(r"\d+\.\d+\.\d+", version) is not None, f"{version}: folder name is a version")
    paths = [folder / name for name in FILES]
    for path in paths:
        check(path.is_file(), f"{version}: {path.name} exists")
    if not all(path.is_file() for path in paths):
        return failures
    v, i, l = (scalars(path) for path in paths)

    # All three files name the same package, version and manifest schema version.
    for name, data in (("version", v), ("installer", i), ("locale", l)):
        check(data.get("PackageIdentifier") == PACKAGE, f"{version}: {name}: PackageIdentifier is {PACKAGE}")
        check(data.get("PackageVersion") == version, f"{version}: {name}: PackageVersion is {version}")
        check(bool(data.get("ManifestVersion")) and data.get("ManifestVersion") == v.get("ManifestVersion"),
              f"{version}: {name}: ManifestVersion matches the others")
    check(v.get("ManifestType") == "version", f"{version}: version: ManifestType is version")
    check(i.get("ManifestType") == "installer", f"{version}: installer: ManifestType is installer")
    check(l.get("ManifestType") == "defaultLocale", f"{version}: locale: ManifestType is defaultLocale")
    check(v.get("DefaultLocale") == "en-US" and l.get("PackageLocale") == "en-US", f"{version}: default locale is en-US")

    # The installer: per-user Inno Setup, the release's versioned URL, a real SHA-256, same AppId.
    text = paths[1].read_text(encoding="utf-8")
    check(i.get("InstallerType") == "inno", f"{version}: installer: InstallerType is inno (installer.iss is Inno Setup)")
    check(i.get("Scope") == "user", f"{version}: installer: Scope is user (PrivilegesRequired=lowest)")
    check(i.get("MinimumOSVersion") == "10.0.17763.0", f"{version}: installer: MinimumOSVersion matches MinVersion in installer.iss")
    check(re.search(r"Architecture:\s*x64\b", text) is not None, f"{version}: installer: Architecture is x64")
    check(f"InstallerUrl: {RELEASES}/v{version}/{INSTALLER_NAME}\n" in text, f"{version}: installer: InstallerUrl is the versioned release asset, not /latest/")
    sha = re.search(r"InstallerSha256:\s*([0-9A-Fa-f]{64})\s*$", text, re.M)
    check(sha is not None, f"{version}: installer: InstallerSha256 is 64 hex characters")
    check(i.get("ProductCode") == f"{app_id}_is1", f"{version}: installer: ProductCode is {app_id}_is1 (AppId in installer.iss)")
    check(all(s in text for s in ("/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART")),
          f"{version}: installer: silent switches cover /VERYSILENT /SUPPRESSMSGBOXES /NORESTART")
    check(i.get("UpgradeBehavior") == "install", f"{version}: installer: UpgradeBehavior is install")

    # The locale.
    check(l.get("PackageName") == "October Lantern", f"{version}: locale: PackageName is October Lantern")
    check(l.get("Publisher") == "October" and l.get("PublisherUrl") == "https://october.dev",
          f"{version}: locale: Publisher and PublisherUrl match installer.iss")
    check(l.get("License") == "Apache-2.0", f"{version}: locale: License is Apache-2.0 (LICENSE)")
    check(l.get("Copyright") == copyright_line, f"{version}: locale: Copyright is NOTICE's line ({copyright_line})")
    check(bool(l.get("ShortDescription")), f"{version}: locale: ShortDescription is set")

    # The hash is what the release publishes (network).
    if online and sha:
        try:
            with urllib.request.urlopen(f"{RELEASES}/v{version}/{INSTALLER_NAME}.sha256", timeout=30) as r:
                published = r.read().decode().split()[0].lower()
            check(sha.group(1).lower() == published, f"{version}: installer: InstallerSha256 equals the release's .sha256")
        except (OSError, ValueError, IndexError) as error:
            check(False, f"{version}: couldn't read the release's .sha256 ({error})")

    # winget's own strict validation, when winget is installed.
    winget = shutil.which("winget") if validate else None
    if winget:
        try:
            result = subprocess.run([winget, "validate", "--manifest", str(folder)], capture_output=True, text=True, timeout=120)
            check(result.returncode == 0, f"{version}: winget validate passes" + ("" if result.returncode == 0 else f"\n{result.stdout}{result.stderr}"))
        except subprocess.TimeoutExpired:
            check(False, f"{version}: winget validate finished within 120 s")
    elif validate:
        say("skip  winget validate (winget not installed)")
    return failures


# Each break must be caught: (what is wrong, file, pattern, replacement).
BREAKS = (
    ("InstallerUrl points at /latest/", 1, r"/releases/download/v[\d.]+/", "/releases/latest/download/"),
    ("InstallerSha256 is one character short", 1, r"(InstallerSha256: [0-9A-Fa-f]{63})[0-9A-Fa-f]", r"\1"),
    ("Scope is machine", 1, r"Scope: user", "Scope: machine"),
    ("ProductCode is not the installer's", 1, r"_is1'", "_is2'"),
    ("silent switches are gone", 1, r"/VERYSILENT", "/QUIET"),
    ("locale PackageVersion is another version", 2, r"PackageVersion: [\d.]+", "PackageVersion: 0.0.1"),
    ("Copyright isn't NOTICE's", 2, r"Copyright: .+", "Copyright: Copyright (c) Someone"),
)


def self_test(newest):
    """Breaks a copy of the newest manifest each way in BREAKS; the checker must notice every one."""
    print(f"\nself-test on {newest.name}")
    escaped = []
    quiet = lambda *_: None
    with tempfile.TemporaryDirectory() as tmp:
        clean = pathlib.Path(tmp) / newest.name
        shutil.copytree(newest, clean)
        if check_folder(clean, validate=False, say=quiet):
            print("FAIL  an unchanged copy should pass")
            escaped.append("unchanged copy")
        for name, index, pattern, replacement in BREAKS:
            broken = pathlib.Path(tmp) / f"broken{index}" / newest.name
            shutil.copytree(newest, broken)
            target = broken / FILES[index]
            before = target.read_text(encoding="utf-8")
            after = re.sub(pattern, replacement, before)
            if after == before:
                print(f"FAIL  self-test setup: '{name}' changed nothing")
                escaped.append(name)
                continue
            target.write_text(after, encoding="utf-8")
            caught = bool(check_folder(broken, validate=False, say=quiet))
            print(("ok    " if caught else "FAIL  ") + f"caught: {name}")
            if not caught:
                escaped.append(name)
            shutil.rmtree(broken.parent)
    return escaped


def main():
    online = "--online" in sys.argv
    folders = sorted((p for p in WINGET.glob("*") if p.is_dir()), key=lambda p: [int(n) if n.isdigit() else 0 for n in p.name.split(".")])
    if not folders:
        raise SystemExit("FAIL  no manifest folders in windows/winget")
    failures = []
    for folder in folders:
        failures += check_folder(folder, online=online)
    engine = re.search(r'^version = "(.+)"', (ROOT / "engine" / "Cargo.toml").read_text(), re.M).group(1)
    if folders[-1].name != engine:
        print(f"note  engine is {engine}; the newest manifest is {folders[-1].name} (a manifest follows its release)")
    failures += self_test(folders[-1])
    if failures:
        raise SystemExit(f"\n{len(failures)} check(s) failed")
    print("\nwinget manifest checks passed")


if __name__ == "__main__":
    main()
