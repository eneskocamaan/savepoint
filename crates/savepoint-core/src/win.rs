//! Small safe wrappers over the Win32 calls the engine needs.

use std::path::PathBuf;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Registry::{
    HKEY, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows::core::{GUID, HSTRING, PCWSTR, PWSTR};

pub use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
pub use windows::Win32::UI::Shell::{FOLDERID_Documents, FOLDERID_LocalAppDataLow};

pub fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let raw = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let path = raw.to_string().ok();
        CoTaskMemFree(Some(raw.0 as *const _));
        path.map(PathBuf::from)
    }
}

pub fn reg_get_string(root: HKEY, subkey: &str, value: &str) -> Option<String> {
    let subkey = HSTRING::from(subkey);
    let value = HSTRING::from(value);
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(root, &subkey, &value, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len))
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let chars = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..chars]))
}

pub fn reg_set_string(root: HKEY, subkey: &str, value: &str, data: &str) -> bool {
    let subkey = HSTRING::from(subkey);
    let value = HSTRING::from(value);
    let wide: Vec<u16> = data.encode_utf16().chain(Some(0)).collect();
    let status = unsafe {
        RegSetKeyValueW(root, &subkey, &value, REG_SZ.0, Some(wide.as_ptr().cast()), (wide.len() * 2) as u32)
    };
    status == ERROR_SUCCESS
}

pub fn reg_delete_value(root: HKEY, subkey: &str, value: &str) {
    let subkey = HSTRING::from(subkey);
    let value = HSTRING::from(value);
    unsafe {
        let _ = RegDeleteKeyValueW(root, &subkey, PCWSTR(value.as_ptr()));
    }
}

/// Local wall-clock time as `YYYYMMDD-HHMMSS`, used for backup folder names.
pub fn local_timestamp() -> String {
    let t = unsafe { GetLocalTime() };
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

/// Names of the direct subkeys of a registry key.
pub fn reg_subkeys(root: HKEY, subkey: &str) -> Vec<String> {
    use windows::Win32::System::Registry::{KEY_READ, RegCloseKey, RegEnumKeyExW, RegOpenKeyExW};
    let mut names = Vec::new();
    let subkey = HSTRING::from(subkey);
    let mut key = HKEY::default();
    if unsafe { RegOpenKeyExW(root, &subkey, None, KEY_READ, &mut key) } != ERROR_SUCCESS {
        return names;
    }
    for index in 0.. {
        let mut buf = [0u16; 256];
        let mut len = buf.len() as u32;
        let status =
            unsafe { RegEnumKeyExW(key, index, Some(PWSTR(buf.as_mut_ptr())), &mut len, None, None, None, None) };
        if status != ERROR_SUCCESS {
            break;
        }
        names.push(String::from_utf16_lossy(&buf[..len as usize]));
    }
    unsafe {
        let _ = RegCloseKey(key);
    }
    names
}

/// Roots of the fixed (non-removable, non-network) drives, like `C:\`.
pub fn fixed_drives() -> Vec<String> {
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    const DRIVE_FIXED: u32 = 3;
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| format!("{}:\\", (b'A' + i) as char))
        .filter(|root| unsafe { GetDriveTypeW(&HSTRING::from(root.as_str())) } == DRIVE_FIXED)
        .collect()
}
