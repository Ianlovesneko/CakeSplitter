# Desktop Installation

## Private preview only

`v0.7.0` has no public installer or download. The steps below are retained
for maintainers using the separately preserved v0.6.0 private preview artifact;
they are not public installation instructions.

1. Obtain the private CakeSplitter Desktop v0.6.0 NSIS installer and its
   private SHA-256 manifest through the project owner.
2. Compare the local installer hash:

   ```powershell
   Get-FileHash '.\CakeSplitter Desktop_0.6.0_x64-setup.exe' -Algorithm SHA256
   ```

3. Run the installer. It installs for the current user and should not request
   administrator elevation.
4. If Windows SmartScreen shows an unknown-publisher warning, verify that the
   filename and SHA-256 match the release before choosing whether to continue.
5. Launch **CakeSplitter Desktop** from the Start menu.

The default install directory is:

```text
%LOCALAPPDATA%\CakeSplitter Desktop
```

The private v0.6.0 installer is not signed and does not claim Trusted Publisher status.
Microsoft Edge WebView2 must already be available; the installer does not fetch
or silently install it.

## Build from source

Install Rust 1.85+, the Windows MSVC build tools, Node.js 20.19+ or 22.12+, npm,
and WebView2. From the repository root:

```powershell
npm ci
npm --workspace @cakesplitter/desktop run tauri:build -- --bundles nsis
```

Tauri writes generated build output under the selected Cargo target directory.
Do not commit that output.

## Uninstall

Use Windows **Installed apps** or run the installed uninstaller. Uninstall
removes the executable, Start menu shortcut, and installer registration.

Uninstall intentionally preserves local task data at:

```text
%LOCALAPPDATA%\io.cakesplitter.desktop
```

This allows a later reinstall to recover compatible tasks safely. To remove it,
first close every CakeSplitter process, confirm that no work needs recovery,
then delete that directory manually. Uninstall and Clear All never delete Slice
packages or rebuilt output files chosen by the user.

## Reinstall and state compatibility

Reinstalling a compatible development build reuses checksummed task state
without duplicating records. Invalid rows are quarantined within bounded limits.
A database with an
unsupported future schema is preserved as renamed local evidence and a clean
current store is created; CakeSplitter does not reinterpret unknown state.
