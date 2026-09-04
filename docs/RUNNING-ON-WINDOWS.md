# Running on Windows

How to build and run the Invar app on Windows 10/11, from a clean
machine to a shipped installer.

> **Testing status.** The app is developed on macOS. The two platform-specific
> code paths (config-file permissions, shutdown signal handling) were compiled
> and checked for Windows, and nothing in the codebase uses POSIX paths or
> shells — but **no run of the finished app on Windows has been observed yet**.
> Treat the first Windows run as a smoke test, not a formality. Anything that
> does go wrong is likely in the [Troubleshooting](#troubleshooting) table.

---

## 1. Prerequisites

Four things, in this order. All commands are **PowerShell**.

| | Why |
|---|---|
| Visual Studio Build Tools 2022 (C++) | Rust's MSVC toolchain needs `link.exe` and the Windows SDK |
| Rust (stable, MSVC) | the backend |
| Node.js LTS (20+) | only to run the Tauri CLI — there is no frontend build step |
| WebView2 Runtime | the window the UI renders in. **Already present on Windows 11** and on most patched Windows 10 |

### The quick way — winget

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

```powershell
winget install --id Rustlang.Rustup
```

```powershell
winget install --id OpenJS.NodeJS.LTS
```

```powershell
winget install --id Git.Git
```

Windows 10 only — Windows 11 already has it:

```powershell
winget install --id Microsoft.EdgeWebView2Runtime
```

**Close and reopen PowerShell** afterwards so the new `PATH` takes effect.

### Manual downloads, if winget is unavailable

- Build Tools — <https://visualstudio.microsoft.com/visual-cpp-build-tools/>
  In the installer, tick **Desktop development with C++**. That workload is the
  requirement; the full Visual Studio IDE is not needed.
- Rust — <https://rustup.rs> (take the default `x86_64-pc-windows-msvc` host)
- Node.js LTS — <https://nodejs.org>
- WebView2 Evergreen Bootstrapper —
  <https://developer.microsoft.com/microsoft-edge/webview2/>

### Confirm the toolchain

```powershell
rustc --version; cargo --version; node --version; npm --version
```

Take the current stable Rust (whatever `rustup` installs today) and Node 20 or
newer. The crate declares no minimum Rust version, so stable is the safe
choice. Then confirm Rust is on the MSVC toolchain, not GNU:

```powershell
rustup show
```

The default host triple must read `x86_64-pc-windows-msvc`. If it says
`-gnu`, switch it:

```powershell
rustup default stable-x86_64-pc-windows-msvc
```

---

## 2. Get the code

```powershell
git clone https://github.com/virtualFrog/Invar.git
```

```powershell
cd Invar
```

```powershell
npm install
```

`npm install` pulls exactly one package — the Tauri CLI. The frontend is plain
HTML/CSS/JS with no build step, so there is nothing to bundle or transpile.

---

## 3. Run it

```powershell
npm run tauri dev
```

The first build compiles the whole Rust dependency tree and takes roughly
**5–15 minutes**. Later runs are seconds. The app window opens by itself.

On first launch it will say *"No vCenter configured yet"*. Click **Settings**,
add a host, username and password, then **Save** — it queries immediately.

---

## 4. Configure without the UI (optional)

Settings are stored here:

```
%APPDATA%\ch.soultec.invar\config.json
```

which expands to `C:\Users\<you>\AppData\Roaming\ch.soultec.invar\`.

Open the folder with:

```powershell
explorer "$env:APPDATA\ch.soultec.invar"
```

The file holds a **list** of connections — add as many vCenters as you like and
every sheet aggregates across all of them, tagging each row with its source in
the `VI SDK Server` column:

```json
{
  "connections": [
    {
      "host": "vcf-mgmt-vc91.vcf.soultec.lab",
      "username": "administrator@vsphere.local",
      "password": "<password>",
      "skip_cert_verify": true
    }
  ]
}
```

`skip_cert_verify` must stay `true` for lab vCenters with self-signed
certificates.

> **Credentials on Windows are less protected than on macOS/Linux.** On those
> platforms the app chmods this file to `0600`. Windows has no equivalent step,
> so the file relies on the default ACL of your roaming profile — which keeps
> other standard users out, but not local administrators. Don't store
> production vCenter credentials on a shared or lab machine.

---

## 5. Build an installer

```powershell
npm run tauri build
```

Output lands in:

```
src-tauri\target\release\bundle\msi\
src-tauri\target\release\bundle\nsis\
```

The files are named from the product name and version in `tauri.conf.json` —
currently *Invar* 0.1.0, so expect something close to
`Invar_0.1.0_x64_en-US.msi` and
`Invar_0.1.0_x64-setup.exe`.

Both are produced because `tauri.conf.json` sets `"targets": "all"`. The MSI is
better for Group Policy or Intune deployment; the NSIS `-setup.exe` is the
friendlier double-click installer.

A release build takes longer than a dev build — budget 10–20 minutes the first
time.

**Tauri only builds for the platform it runs on.** This Windows installer must
be built on Windows; a Mac cannot produce it. (Cross-compiling is blocked well
before linking — the Windows resource compiler isn't available on macOS.)

**The installer is unsigned**, so SmartScreen will show *"Windows protected your
PC"* on first run. Choose **More info → Run anyway**, or sign the binary with a
code-signing certificate if it is going to be distributed.

---

## 6. Verify against vCenter from the command line

Useful when the app shows no data and you need to know whether the problem is
the app or the network.

### REST login smoke test

PowerShell 7+:

```powershell
$cred = Get-Credential -UserName "administrator@vsphere.local" -Message "vCenter"
```

```powershell
Invoke-RestMethod -Method Post -Uri "https://vcf-mgmt-vc91.vcf.soultec.lab/rest/com/vmware/cis/session" -Credential $cred -SkipCertificateCheck
```

A session token comes back as `{"value": "..."}`. Windows PowerShell 5.1 has no
`-SkipCertificateCheck`; use PowerShell 7, or `curl.exe` (shipped with Windows
10 1803+ and later):

```powershell
curl.exe -sk -X POST -u "administrator@vsphere.local:PASSWORD" https://vcf-mgmt-vc91.vcf.soultec.lab/rest/com/vmware/cis/session
```

### Run a sheet without the UI

The repo has console examples that exercise the same core fetch functions the
app uses — handy for isolating a data problem from a UI problem:

```powershell
cd src-tauri
```

```powershell
$env:VC_HOST="vcf-mgmt-vc91.vcf.soultec.lab"; $env:VC_USER="administrator@vsphere.local"; $env:VC_PASS="<password>"
```

```powershell
cargo run --example verify vInfo
```

Swap `vInfo` for `vHost`, `vDisk`, `vSnapshot` or `vHealth`. Two more examples:

```powershell
cargo run --example export -- C:\Temp\inventory.xlsx
```

```powershell
cargo run --example concurrent
```

`export` writes the full RVTools-format workbook; `concurrent` proves the
session cache logs in once when every sheet is fetched at the same time.

Clear the password out of the shell when you're done:

```powershell
Remove-Item Env:VC_PASS
```

### Run the unit tests

```powershell
cargo test
```

Nine tests, no vCenter required — they run against captured vCenter XML.

---

## Windows-specific behaviour worth knowing

**Session cleanup on exit.** vCenter sessions linger for ~30 minutes after the
app goes away unless it logs out explicitly. Closing the window, or Ctrl-C in
the terminal, triggers a clean logout. **Killing the process from Task Manager
does not** — that session stays open until vCenter times it out. On Linux and
macOS the app also handles SIGTERM; Windows has no equivalent, so avoid
force-killing it during testing. To check for strays, count `<UserSession>`
entries as described in `LAB-ENVIRONMENT.md`.

**Fonts.** The xlsx export uses Verdana 9pt to match RVTools. Verdana ships with
Windows, so exports render exactly as RVTools' do — arguably more faithfully
than on macOS.

**Excel.** Exported workbooks open in Excel with frozen panes at `B2` and
AutoFilter already applied, matching RVTools' layout. See
`RVTOOLS-SHEETS-AND-COLUMNS.md` for the full format comparison.

**Antivirus and build speed.** Real-time scanning of `target\` slows Rust builds
noticeably. If builds crawl, exclude the repo's `target` directory in Windows
Security → Virus & threat protection → Exclusions.

**Line endings.** The repo has no `.gitattributes`, so Git may check files out
with CRLF. Nothing in the build cares.

---

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `error: linker 'link.exe' not found` | C++ build tools missing | Install Build Tools with the **Desktop development with C++** workload, then reopen PowerShell |
| `error: Microsoft Visual C++ 14.0 or greater is required` | same | as above |
| Build works, window is blank or never opens | WebView2 Runtime missing | Install the Evergreen Bootstrapper (Windows 10) |
| `npm : command not found` after installing Node | `PATH` not refreshed | Close and reopen PowerShell |
| Want to run without the Tauri CLI | the crate has a single binary, so `cargo run` works from `src-tauri\` | Prefer `npm run tauri dev` — it also watches for changes and rebuilds |
| `The system cannot find the path specified` from `npm run tauri build` | running from `src-tauri\` | Run npm scripts from the **repo root**; only `cargo` commands run from `src-tauri\` |
| REST/SOAP calls fail with a certificate error | self-signed vCenter cert | Ensure `skip_cert_verify` is `true` in `config.json` |
| App shows a warning banner naming one vCenter | that server was unreachable; the others still returned data | Check name resolution and credentials for the named host — the row counts shown exclude it |
| `vSwitch` / `vPort` empty, `vHBA` WWN blank | not bugs — see `LAB-ENVIRONMENT.md` | no action |
| SmartScreen blocks the installer | unsigned binary | **More info → Run anyway**, or code-sign it |
| Builds are extremely slow | antivirus scanning `target\` | Add an exclusion for the repo directory |

---

## Quick reference

| Task | Command | Run from |
|---|---|---|
| Install deps | `npm install` | repo root |
| Run the app | `npm run tauri dev` | repo root |
| Build installers | `npm run tauri build` | repo root |
| Unit tests | `cargo test` | `src-tauri\` |
| One sheet, no UI | `cargo run --example verify vHost` | `src-tauri\` |
| Full xlsx export | `cargo run --example export -- C:\Temp\out.xlsx` | `src-tauri\` |
| Config file | `explorer "$env:APPDATA\ch.soultec.invar"` | anywhere |
