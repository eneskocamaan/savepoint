# Contributing to Savepoint

Thanks for helping! Bug reports, ideas, translations and code are all welcome. This page explains how to get set up and what a good contribution looks like.

By taking part you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md). Contributions are accepted under the project's [MIT License](LICENSE).

## Reporting a bug

Open an [issue](https://github.com/eneskocamaan/savepoint/issues/new/choose) and fill in the form. The most useful things to include:

- Savepoint version (shown at the bottom of Settings) and your Windows version.
- What you did, what you expected, and what happened.
- The game, and the store it came from (Steam, Epic, GOG, …).
- The end of the log: `%LOCALAPPDATA%\Savepoint\savepoint.log`. It contains game names and folder paths on your PC; remove anything you do not want to share.

Security problems should **not** go into a public issue; see [SECURITY.md](SECURITY.md).

## Suggesting a feature

Open an issue with the "Feature request" form. Savepoint deliberately stays small: it should cost nothing while games run and need no setup. Ideas that keep it that way have the best chance.

## Development setup

- Windows 10 or 11.
- [Rust](https://rustup.rs) 1.90 or newer. The MSVC toolchain (default on Windows) needs Visual Studio Build Tools with "Desktop development with C++". The GNU toolchain also works if `gcc` and `windres` are on `PATH` (for example from w64devkit); its builds need `WebView2Loader.dll` next to the exes, which `scripts\package.ps1` copies.
- Optional: [Inno Setup 6](https://jrsoftware.org/isinfo.php) to build the installer.
- No Node.js: the window is plain HTML, CSS and JavaScript in `ui/web`, embedded at build time.

```powershell
git clone https://github.com/eneskocamaan/savepoint
cd savepoint
cargo build                      # debug build
cargo run -p savepointd          # start the service (opens the window on first run)
cargo test --workspace
```

Savepoint keeps its own files in `%LOCALAPPDATA%\Savepoint` (`config.json`, the cached manifest, `games.json`, `savepoint.log`). Delete that folder to test a first start. Only one service runs per Windows user, so quit a running copy (tray menu → Quit) before starting yours.

## Project layout

| Path | What lives there |
|---|---|
| `crates/savepoint-core` | The engine: manifest download and parsing, game detection (`scan.rs`, `installs.rs`), backup and restore (`backup.rs`, `portable.rs`), icons. No background work of its own. |
| `crates/savepoint-ipc` | Requests and replies between the window and the service, sent over a per-user named pipe as one JSON line each. |
| `crates/savepointd` | The service: tray icon, game watcher, backup worker, pipe server. |
| `ui/src-tauri` | The window's Rust side (Tauri): placement, file dialogs, forwarding requests to the service. |
| `ui/web` | The window's HTML, CSS and JavaScript; translations in `ui/web/lang`. |
| `installer` | Inno Setup script. |
| `scripts` | Packaging and third-party license collection. |

Guiding rules, in order: never lose or damage a save; cost nothing while a game runs; need no setup.

## Pull requests

1. Fork the repository and create a branch from `main` (`fix-restore-dialog`, `add-german`, …).
2. Keep the change focused; one pull request per fix or feature.
3. Before pushing, run:

   ```powershell
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```

   The same checks run automatically on every pull request.
4. Add or update tests for engine changes (`crates/savepoint-core`), especially anything touching backup or restore.
5. For window changes, include a screenshot in the pull request.
6. Add a line under "Unreleased" in [CHANGELOG.md](CHANGELOG.md) if users would notice the change.

Write code like the code around it: small functions, comments that explain *why*, no new dependency unless it clearly pays for itself.

## Translations

The window, the tray menu and the installer are translated separately:

1. **Window:** copy `ui/web/lang/en.json` to `ui/web/lang/<code>.json` (for example `de.json`) and translate the values; keep the keys and the `{placeholders}` as they are. Keys ending in `.one` / `.other` are plural forms; languages without plural forms only need `.other`. Then add the language to `LANGUAGES` at the top of `ui/web/i18n.js`, using its own name for itself ("Deutsch").
2. **Tray menu:** add the code to `SUPPORTED` and a row to `TRAY` in `crates/savepointd/src/i18n.rs`.
3. **Installer (optional):** if Inno Setup ships the language, add it under `[Languages]` in `installer/savepoint.iss` and translate `NeedWebView2` under `[CustomMessages]`.

Missing keys fall back to English, so a partial translation is fine to start with.

## Releases (maintainers)

1. Update the version in `Cargo.toml` (`[workspace.package]`), and in `ui/src-tauri/tauri.conf.json`.
2. Move the "Unreleased" notes in `CHANGELOG.md` under the new version with today's date.
3. Commit, then tag and push: `git tag v0.2.0 && git push origin main v0.2.0`.
4. The Release workflow builds the portable zip, the installer and `SHA256SUMS.txt`, and publishes them with the changelog notes.
