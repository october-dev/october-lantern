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
