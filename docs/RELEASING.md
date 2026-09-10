# Releasing

Tagging is the whole release process. `.github/workflows/release.yml` fires on a
`v*` tag, builds macOS, Windows and Linux in parallel, attaches everything to a
draft GitHub Release, and publishes it when all three succeed.

Everything below the first section is one-time setup.

---

## Cutting a release

1. Bump the version in **three** files, which must agree:
   - `src-tauri/tauri.conf.json` (`version`)
   - `src-tauri/Cargo.toml` (`[package] version`)
   - `package.json` (`version`)
2. Commit the bump, then tag and push:

   ```bash
   git commit -am "Release 0.2.0"
   git tag v0.2.0
   git push origin main --tags
   ```

3. Watch the run. It produces:

   | Platform | Artifacts |
   |---|---|
   | macOS | `Invar_<version>_universal.dmg`, signed and notarized, Apple silicon and Intel in one file |
   | Windows | `Invar_<version>_x64_en-US.msi` and `Invar_<version>_x64-setup.exe`, **unsigned** |
   | Linux | `.deb`, `.rpm`, `.AppImage` |
   | All three | `invar-export-<version>-<platform>`, the headless exporter as a standalone binary |

If the macOS secrets below are absent the build still succeeds, but the `.dmg`
is only ad-hoc signed, and anyone who downloads it is told the app "is damaged
and can't be opened". That message is Gatekeeper refusing an unnotarized
download, not a corrupt file.

---

## One-time: Apple signing and notarization

Six repository secrets. Add them at
**Settings > Secrets and variables > Actions > New repository secret**
in the GitHub repo (`https://github.com/virtualFrog/Invar/settings/secrets/actions`).

You need an Apple Developer Program membership (99 USD/year). A free Apple ID
cannot issue the certificate this needs.

### 1. `APPLE_TEAM_ID`

Your 10-character team identifier, e.g. `A1B2C3D4E5`.

Sign in at <https://developer.apple.com/account>. It is shown under
**Membership details** as *Team ID*.

### 2. `APPLE_SIGNING_IDENTITY`

The full certificate name, including the team ID in brackets:

```
Developer ID Application: soulTec AG (A1B2C3D4E5)
```

Getting the certificate, if you do not already have one:

1. On your Mac, open **Keychain Access**, menu
   **Keychain Access > Certificate Assistant > Request a Certificate From a
   Certificate Authority**.
2. Enter your email and name, choose **Saved to disk**, and save the
   `.certSigningRequest` file.
3. At <https://developer.apple.com/account/resources/certificates/list>, click
   **+**, choose **Developer ID Application** (this is the one for software
   distributed outside the Mac App Store, *not* "Mac Development" and *not*
   "Mac App Distribution"), upload the request file, and download the resulting
   `.cer`.
4. Double-click the `.cer` to install it into your login keychain.

Then read the exact identity string back:

```bash
security find-identity -v -p codesigning
```

Copy the quoted name from the `Developer ID Application:` line, in full.

### 3. `APPLE_CERTIFICATE`

The same certificate exported as a base64-encoded `.p12`.

1. In **Keychain Access**, find the **Developer ID Application** certificate,
   expand it so you can see the private key underneath, select **both** rows,
   right-click and choose **Export 2 items**.
2. Save as **Personal Information Exchange (.p12)** and set a password when
   asked. Exporting the certificate without its private key produces a file
   that cannot sign anything.
3. Encode it:

   ```bash
   base64 -i Certificates.p12 | pbcopy
   ```

   That puts the value on your clipboard. Paste it as the secret.

### 4. `APPLE_CERTIFICATE_PASSWORD`

The password you set when exporting the `.p12` in step 3. Not your Apple ID
password.

### 5. `APPLE_ID`

The email address of the Apple ID that owns the developer membership, e.g.
`you@example.com`.

### 6. `APPLE_PASSWORD`

An **app-specific password**, not your account password. Notarization is
refused if you use the account password.

1. Sign in at <https://account.apple.com>.
2. Under **Sign-In and Security**, choose **App-Specific Passwords**.
3. Generate one, label it something like `invar-notarization`, and copy the
   `xxxx-xxxx-xxxx-xxxx` value. It is shown once.

### Checking it worked

Download the `.dmg` from the finished release on a Mac that did not build it,
then:

```bash
spctl -a -vvv -t install /Volumes/Invar/Invar.app
# expect: accepted, source=Notarized Developer ID
```

`source=Unnotarized Developer ID` means signing worked and notarization did not:
look at the notarization step's log in the workflow run.

---

## Not set up: Windows code signing

Released `.msi` and `.exe` files are unsigned, so Windows shows
*"Windows protected your PC"* on first run, and the user has to choose
**More info > Run anyway**.

Fixing it needs an OV or EV code-signing certificate from a commercial CA
(DigiCert, Sectigo and others; roughly 200 to 600 CHF/year, and EV certificates
ship on a hardware token that a GitHub-hosted runner cannot read). If you buy
one, wire it up through `bundle.windows.certificateThumbprint` and
`signCommand` in `tauri.conf.json`, or use an Azure Trusted Signing account,
which does work from a hosted runner.

**The NSIS `-setup.exe` also carries an empty `CompanyName`**, which makes that
SmartScreen dialog read worse than it needs to — it is the field the prompt
falls back to with no signature. This is **not** a configuration mistake:
`bundle.publisher` is set, and Tauri uses it for the Add/Remove Programs
`Publisher` entry, but the NSIS template it generates (`tauri-cli` 2.11.4,
`target/release/nsis/x64/installer.nsi`) emits `VIAddVersionKey` for
`ProductName`, `FileDescription`, `LegalCopyright`, `FileVersion` and
`ProductVersion` only — never `CompanyName`. The MSI and `invar.exe` are both
correct. Closing it means either an upstream fix or forking the whole installer
template via `bundle.windows.nsis.template` to add one line, which is a poor
trade until signing is wired up and the dialog stops appearing anyway.

---

## Not set up: auto-update

There is no updater. Users get new versions by downloading them. Adding one
means the `tauri-plugin-updater`, a signing keypair of its own
(`tauri signer generate`), and somewhere to host the update manifest.
