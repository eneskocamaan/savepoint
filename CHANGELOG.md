# Changelog

All notable changes to Savepoint are listed here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-10-24

First public release.

### Added

- Background service with a tray icon that backs up a game's saves when the game is closed, only if something changed.
- Game detection from the Ludusavi Manifest, Steam libraries, Epic Games, GOG Galaxy, the Xbox app, installed programs registered in Windows, common game folders, and Steam emulators (`steam_appid.txt`, emulator save folders).
- Versioned plain-folder backups with hard links for unchanged files and a configurable number of backups per game.
- Restore with a preview of what will be overwritten and an automatic backup of the current saves first.
- Damaged-save protection: empty or shrunken saves are flagged and the last good backup is always kept.
- Backups that can be restored on another PC (paths stored relative to Documents, AppData, the install folder, …).
- Red error bar for failed backups (disk full, unreachable folder, locked or inaccessible files) and the game list scan.
- Window opening above the taskbar: game grid with icons, backup history, settings, hidden games, manual "Add game".
- English, Turkish and Spanish.
- Installer (per user, no admin rights, optional start with Windows) and portable zip.

[Unreleased]: https://github.com/eneskocamaan/savepoint/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/eneskocamaan/savepoint/releases/tag/v0.1.0
