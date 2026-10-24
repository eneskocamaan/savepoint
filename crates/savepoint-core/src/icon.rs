//! Game icons: pulled out of the game's exe once, stored as small PNGs.

use crate::fsutil;
use std::ffi::c_void;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, GetObjectW, HBITMAP,
    HGDIOBJ, ReleaseDC,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO, PrivateExtractIconsW};

/// Pixel size of stored icons; shown smaller, so they stay sharp on HiDPI.
const SIZE: i32 = 64;

pub fn path(data_dir: &Path, game: &str) -> PathBuf {
    data_dir.join("icons").join(format!("{}.png", fsutil::safe_name(game)))
}

/// Extracts the exe's icon and saves it for `game`. Returns false if the exe has none.
pub fn save_from_exe(data_dir: &Path, game: &str, exe: &Path) -> io::Result<bool> {
    let Some((w, h, rgba)) = extract(exe) else { return Ok(false) };
    fsutil::write_atomic(&path(data_dir, game), &encode_png(w, h, &rgba))?;
    Ok(true)
}

pub fn read_base64(data_dir: &Path, game: &str) -> io::Result<String> {
    Ok(base64(&fs::read(path(data_dir, game))?))
}

fn extract(exe: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let mut name = [0u16; 260];
    for (dst, src) in name.iter_mut().zip(exe.to_string_lossy().encode_utf16().take(259)) {
        *dst = src;
    }
    let mut icons = [HICON::default()];
    let found = unsafe { PrivateExtractIconsW(&name, 0, SIZE, SIZE, Some(&mut icons), None, 0) };
    if found == 0 || found == u32::MAX || icons[0].is_invalid() {
        return None;
    }
    let pixels = unsafe { icon_pixels(icons[0]) };
    unsafe {
        let _ = DestroyIcon(icons[0]);
    }
    pixels
}

/// Reads an icon as top-down RGBA, using its mask when it has no alpha channel.
unsafe fn icon_pixels(icon: HICON) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let mut info = ICONINFO::default();
        GetIconInfo(icon, &mut info).ok()?;
        let result = read_color_and_mask(&info);
        let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
        let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
        result
    }
}

unsafe fn read_color_and_mask(info: &ICONINFO) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        if info.hbmColor.is_invalid() {
            return None;
        }
        let mut bmp = BITMAP::default();
        let got =
            GetObjectW(HGDIOBJ(info.hbmColor.0), size_of::<BITMAP>() as i32, Some(&mut bmp as *mut _ as *mut c_void));
        if got == 0 || bmp.bmWidth <= 0 || bmp.bmHeight <= 0 {
            return None;
        }
        let (w, h) = (bmp.bmWidth, bmp.bmHeight);
        let mut rgba = dib_bgra(info.hbmColor, w, h)?;
        if rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
            // Old-style icon: transparency lives in the 1-bit mask (white = see-through).
            let mask = dib_bgra(info.hbmMask, w, h)?;
            for (px, m) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(mask.as_chunks::<4>().0) {
                px[3] = if m[0] != 0 { 0 } else { 255 };
            }
        }
        for px in rgba.as_chunks_mut::<4>().0 {
            px.swap(0, 2); // BGRA -> RGBA
        }
        Some((w as u32, h as u32, rgba))
    }
}

unsafe fn dib_bgra(bitmap: HBITMAP, w: i32, h: i32) -> Option<Vec<u8>> {
    unsafe {
        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // negative: top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let dc = GetDC(None);
        let lines = GetDIBits(dc, bitmap, 0, h as u32, Some(buf.as_mut_ptr().cast()), &mut header, DIB_RGB_COLORS);
        ReleaseDC(None, dc);
        (lines != 0).then_some(buf)
    }
}

// ---- tiny encoders, so no image or compression crates are needed ----

/// PNG with uncompressed (stored) deflate blocks. Icons are a few KB, so size is no concern.
fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
    for row in rgba.chunks_exact(w as usize * 4) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65535).peekable();
    while let Some(block) = blocks.next() {
        z.push(blocks.peek().is_none() as u8);
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &z);
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n =
            (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_reference() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn checksums_match_reference() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn png_has_valid_structure() {
        let png = encode_png(2, 2, &[255; 16]);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }

    #[test]
    fn extracts_icon_from_real_exe() {
        let exe = Path::new(r"C:\Windows\System32\notepad.exe");
        if !exe.exists() {
            return;
        }
        let (w, h, rgba) = extract(exe).expect("notepad has an icon");
        assert_eq!((w * h * 4) as usize, rgba.len());
        assert!(rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0), "icon is fully transparent");
    }
}
