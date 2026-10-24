//! The few strings the service shows itself (the tray menu). Everything in the
//! panel is translated by the panel, from `ui/web/lang/*.json`.
//!
//! To add a language: add its code to `SUPPORTED`, a row to `TRAY`, and a
//! `lang/<code>.json` file to the panel.

use windows::Win32::Globalization::GetUserDefaultLocaleName;

const SUPPORTED: &[&str] = &["en", "tr", "es"];

/// (language, open, back up everything, quit)
const TRAY: &[(&str, [&str; 3])] = &[
    ("en", ["Open Savepoint", "Back up everything now", "Quit"]),
    ("tr", ["Savepoint'i aç", "Hepsini şimdi yedekle", "Çıkış"]),
    ("es", ["Abrir Savepoint", "Hacer copia de todo ahora", "Salir"]),
];

/// A saved setting is either "auto" or one of the supported codes.
pub fn is_supported(lang: &str) -> bool {
    lang == "auto" || SUPPORTED.contains(&lang)
}

/// Turns the setting into a supported language code ("auto" follows Windows).
pub fn resolve(setting: &str) -> &'static str {
    let wanted = if setting == "auto" { system_language() } else { setting.to_string() };
    SUPPORTED.iter().copied().find(|code| *code == wanted).unwrap_or("en")
}

/// The primary language of the Windows user locale, e.g. "tr" for "tr-TR".
fn system_language() -> String {
    let mut buf = [0u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(&mut buf) };
    let name = String::from_utf16_lossy(&buf[..(len.max(1) - 1) as usize]);
    name.split(['-', '_']).next().unwrap_or("en").to_lowercase()
}

pub fn tray(lang: &str, key: &str) -> &'static str {
    let index = match key {
        "open" => 0,
        "backup_all" => 1,
        _ => 2,
    };
    TRAY.iter().find(|(code, _)| *code == lang).map_or(TRAY[0].1[index], |(_, strings)| strings[index])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_and_falls_back_to_english() {
        assert_eq!(resolve("tr"), "tr");
        assert_eq!(resolve("xx"), "en");
        assert!(is_supported("auto") && is_supported("es") && !is_supported("xx"));
        assert_eq!(tray("tr", "quit"), "Çıkış");
        assert_eq!(tray("zz", "quit"), "Quit");
    }
}
