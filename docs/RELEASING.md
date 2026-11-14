# Releasing October Lantern

Every platform ships the same version, from the same commit, as one release in
[harshsaver/october-lantern-releases](https://github.com/harshsaver/october-lantern-releases):
the Mac DMG and its appcast, the Windows installer and zip, and the Linux bundle. Installed
copies update from there: the Mac through Sparkle's `appcast.xml`, Windows and Linux from the
latest release's files.

## 1. Version

Set the same version in `engine/Cargo.toml` and `desktop/Cargo.toml` (`scripts/check.sh`
refuses a mismatch), update the lock files and the "beta (vX)" line in `docs/PRODUCT.md`:

```sh
cd engine && cargo update -p lantern-engine --offline && cd ..
cd desktop && cargo update -p lantern-desktop --offline && cd ..
```

Commit ("0.x.y") and push to `main`.

## 2. Mac (on a Mac with the Developer ID certificate)

```sh
scripts/release.sh                       # signed, notarized DMG and appcast in build/
scripts/publish.sh "Release notes"        # the GitHub release and the appcast
```

Publish the Mac first: it creates the release with its notes; Windows and Linux add their files
to it.

## 3. Windows and Linux (built by GitHub Actions)

Run the **Windows release bundle** and **Linux release bundle** workflows on `main`
(Actions › the workflow › Run workflow, or `gh workflow run windows-release.yml --ref main`).
Each builds, tests, packages and (Windows) signs, then uploads the files as the run's
artifact. Then, from a checkout of the same commit:

```sh
scripts/publish-desktop.sh windows <run id>
scripts/publish-desktop.sh linux <run id>
```

The script checks the version, the checksums, the update source and (Windows) the signatures
before uploading, and never replaces a published file.

## 4. winget (after the Windows installer is published)

`windows/winget/<version>/` holds the reviewed winget manifest for a release (version, installer
and en-US locale files). It is the copy we check and test here; Windows users only get it once the
same files are merged into [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) at
`manifests/o/October/Lantern/<version>/`, which is a separate pull request there. A manifest can
only be written once its release is published, since it needs the installer's hash.

**Keep `RequireExplicitUpgrade: true` in every manifest.** Lantern updates itself into
`%LOCALAPPDATA%\October Lantern\releases` without changing the version Windows recorded for the
installer, so a bulk `winget upgrade --all` could otherwise install an older catalogue version over
a newer self-updated copy. The flag keeps Lantern out of bulk upgrades for installs winget tracks.

### Review the metadata against its sources

| Manifest field | Must match |
|---|---|
| `PackageVersion` | `engine/Cargo.toml` of the released commit |
| `InstallerUrl` | the release's versioned asset (`.../releases/download/v<version>/October-Lantern-windows-x86_64-setup.exe`), never `latest` |
| `InstallerSha256` | the release's `October-Lantern-windows-x86_64-setup.exe.sha256` |
| `InstallerType: inno`, `Scope: user` | `windows/installer.iss` is Inno Setup with `PrivilegesRequired=lowest` |
| `ProductCode` | `AppId` in `windows/installer.iss` (one `{`), followed by `_is1` |
| `MinimumOSVersion` | `MinVersion` in `windows/installer.iss` |
| `Dependencies` | `Microsoft.VCRedist.2015+.x64`, because `october-lantern.exe` and `lantern-engine.exe` import `VCRUNTIME140.dll` and a clean Windows doesn't have it. Drop it only after the build links the runtime statically and Lantern still starts on a Windows with no Visual C++ runtime |
| `Publisher`, `PackageName` | `AppPublisher`, `AppName` in `windows/installer.iss` |
| `Copyright`, `License` | `NOTICE`, `LICENSE` |
| `ReleaseDate`, `ReleaseNotesUrl` | the release page |

### Validate and test on Windows

On a clean Windows machine ([Windows Sandbox](https://learn.microsoft.com/windows/security/application-security/application-isolation/windows-sandbox/) works, but it ships
without winget: install winget there first, following Microsoft's
[winget-on-Sandbox instructions](https://learn.microsoft.com/en-us/windows/package-manager/winget/#install-winget-on-windows-sandbox),
or use winget-pkgs' [SandboxTest.ps1](https://github.com/microsoft/winget-pkgs/blob/master/doc/tools/SandboxTest.md),
which sets up the environment and tests installation), note the Windows
and winget versions (`ver`, `winget --version`), then:

```powershell
# once, from an administrator shell: allow installing from local manifests
winget settings --enable LocalManifestFiles

# from a normal shell
winget validate --manifest windows\winget\<version>
winget install --manifest windows\winget\<version>          # finishes without installer choices
winget list --name "October Lantern" --exact --scope user --details   # name, publisher, version, per-user location
```

Check the listed version before Lantern's own updater replaces the running copy. winget's silent
install skips the installer's "Open October Lantern" step, so open Lantern from the Start menu and
check that the app and its tray icon appear. Quit Lantern, then:

```powershell
winget uninstall --manifest windows\winget\<version>        # the Apps entry and the Start menu shortcut are gone
```

Repeat from a clean state with `winget install --manifest windows\winget\<version> --silent`. A
local-manifest install isn't tracked under the catalogue id, so don't expect
`winget list October.Lantern` to find it; look it up by name as above. Report any security or
source-agreement prompts.

### Submit, and later versions

For the first submission, open a pull request to microsoft/winget-pkgs with the checked-in files and
the test results. Once `October.Lantern` is in the catalogue, make each new version's manifest with
[wingetcreate](https://github.com/microsoft/winget-create/blob/main/doc/update.md):

```powershell
wingetcreate update October.Lantern --version <version> --urls <InstallerUrl> --out build\winget
```

Review what it generates against the table above (especially `ReleaseDate`, `ReleaseNotesUrl` and
`RequireExplicitUpgrade: true`), copy it to `windows/winget/<version>/`, run the validate and
install tests, then submit the reviewed folder (`wingetcreate submit windows\winget\<version>`,
or a pull request by hand).

After the first catalogue publication, install once from the source (`winget install
October.Lantern`) and confirm that `winget list October.Lantern` matches the installed copy and that
`winget upgrade --all` leaves Lantern out. The local-manifest test above can't check source
tracking.

## Windows code signing (Azure Artifact Signing)

Unsigned builds make Windows show "Windows protected your PC", so `publish-desktop.sh` refuses
them unless `ALLOW_UNSIGNED=1`. To sign with
[Azure Artifact Signing](https://learn.microsoft.com/azure/artifact-signing/) (formerly
Trusted Signing):

0. Check eligibility first: Public Trust validates an organization registered in the US, Canada,
   the EU, the UK or a few other countries (individuals: US and Canada only), with a website and
   two email addresses on its own domain, and Microsoft may require about three years of tax
   history. It needs a pay-as-you-go Azure subscription (Basic is $9.99 a month, billed from the
   day the account is created), and validation takes 1 to 20 business days. If it fails, delete
   the account, and use a `.pfx` from a commercial certificate authority instead (below).
1. In the Azure portal, open the Artifact Signing account and complete **identity validation**,
   then create a **certificate profile** (Public Trust).
2. Create an app registration (Microsoft Entra ID › App registrations) with a client secret, and
   give it the **Artifact Signing Certificate Profile Signer** role on the signing account.
3. In this repository's settings (Secrets and variables › Actions), add:

   | Kind | Name | Value |
   |---|---|---|
   | Variable | `AZURE_SIGNING_ENDPOINT` | the account's endpoint, e.g. `https://eus.codesigning.azure.net/` |
   | Variable | `AZURE_SIGNING_ACCOUNT` | the signing account's name |
   | Variable | `AZURE_SIGNING_PROFILE` | the certificate profile's name |
   | Secret | `AZURE_TENANT_ID` | the app registration's tenant (directory) id |
   | Secret | `AZURE_CLIENT_ID` | the app registration's client (application) id |
   | Secret | `AZURE_CLIENT_SECRET` | the client secret |

The next Windows release run signs every program and the installer
(`scripts/sign-windows.ps1`, using Microsoft's pinned signtool plug-in) and records it in the
build's `release-info.json`. A `.pfx` certificate works instead: the `WINDOWS_CERTIFICATE`
(base64) and `WINDOWS_CERTIFICATE_PASSWORD` secrets.
