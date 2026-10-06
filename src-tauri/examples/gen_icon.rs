//! Generates the 1024x1024 app icon source `icons/src.png` using std only.
//!
//! Design: flat open-book mark on a solid indigo background — two white
//! 270x330 pages separated by a 16px spine seam (rounded outer corners built
//! from rects + circle caps), three indigo text grooves per page, and an
//! amber bookmark ribbon with a V notch hanging from the right page.
//!
//! The PNG stream is encoded by hand (no image/png crates): truecolor RGB
//! IHDR, a single IDAT holding a zlib stream (0x78 0x01) of stored
//! (uncompressed) DEFLATE blocks over scanlines prefixed with filter byte 0,
//! IEND, table-driven CRC-32 per chunk and Adler-32 over the raw data.
//!
//! `cargo run --example gen_icon` (from `src-tauri/`) writes `icons/src.png`.
//! The example does not use the workspace library, so it can also be built
//! standalone: `rustc -O examples/gen_icon.rs -o gen_icon && ./gen_icon out.png`.

#![cfg_attr(test, allow(dead_code))]

use std::env;
use std::fs;
use std::path::PathBuf;

const W: usize = 1024;
const H: usize = 1024;

const BG: [u8; 3] = [0x4F, 0x6B, 0xFF]; // indigo
const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];
const AMBER: [u8; 3] = [0xFF, 0xB0, 0x20];

fn put(buf: &mut [u8], x: i32, y: i32, c: [u8; 3]) {
    if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 {
        return;
    }
    let i = (y as usize * W + x as usize) * 3;
    buf[i..i + 3].copy_from_slice(&c);
}

fn fill_rect(buf: &mut [u8], x: i32, y: i32, w: i32, h: i32, c: [u8; 3]) {
    for yy in y..y + h {
        for xx in x..x + w {
            put(buf, xx, yy, c);
        }
    }
}

fn fill_circle(buf: &mut [u8], cx: i32, cy: i32, r: i32, c: [u8; 3]) {
    let r2 = i64::from(r) * i64::from(r);
    for dy in -r..=r {
        let rem = r2 - i64::from(dy) * i64::from(dy);
        if rem < 0 {
            continue;
        }
        let half = (rem as f64).sqrt() as i32;
        for dx in -half..=half {
            put(buf, cx + dx, cy + dy, c);
        }
    }
}

fn draw(buf: &mut [u8]) {
    let (page_w, page_h, seam) = (270, 330, 16);
    let book_w = page_w * 2 + seam; // 556 -> book spans x 234..790, y 347..677
    let x0 = (W as i32 - book_w) / 2;
    let y0 = (H as i32 - page_h) / 2;
    let y1 = y0 + page_h;
    let rp_x0 = x0 + page_w + seam;
    let rp_x1 = rp_x0 + page_w;
    let r = 28;

    fill_rect(buf, 0, 0, W as i32, H as i32, BG);
    fill_rect(buf, x0, y0, page_w, page_h, WHITE);
    fill_rect(buf, rp_x0, y0, page_w, page_h, WHITE);

    // rounded outer corners: knock out a BG square, restore a white quarter disc
    fill_rect(buf, x0, y0, r, r, BG);
    fill_circle(buf, x0 + r, y0 + r, r, WHITE);
    fill_rect(buf, x0, y1 - r, r, r, BG);
    fill_circle(buf, x0 + r, y1 - r, r, WHITE);
    fill_rect(buf, rp_x1 - r, y0, r, r, BG);
    fill_circle(buf, rp_x1 - r, y0 + r, r, WHITE);
    fill_rect(buf, rp_x1 - r, y1 - r, r, r, BG);
    fill_circle(buf, rp_x1 - r, y1 - r, r, WHITE);

    // shave the discs' 1px overshoot just outside the book bounding box
    fill_rect(buf, x0 - 1, y0 - 1, book_w + 2, 1, BG);
    fill_rect(buf, x0 - 1, y1, book_w + 2, 1, BG);
    fill_rect(buf, x0 - 1, y0, 1, page_h, BG);
    fill_rect(buf, rp_x1, y0, 1, page_h, BG);

    // three text grooves per page (14px tall, wider margin towards the spine)
    let (line_w, line_h, gap) = (178, 14, 62);
    let ty = y0 + (page_h - (3 * line_h + 2 * gap)) / 2;
    for i in 0..3 {
        let yy = ty + i * (line_h + gap);
        fill_rect(buf, x0 + 44, yy, line_w, line_h, BG);
        fill_rect(buf, rp_x0 + 48, yy, line_w, line_h, BG);
    }

    // amber bookmark ribbon on the right page, V notch cut from its tail
    let (bm_w, bm_h) = (56, 120);
    let bm_x = rp_x0 + (page_w - bm_w) / 2;
    fill_rect(buf, bm_x, y0, bm_w, bm_h, AMBER);
    let depth = 34;
    let apex_y = y0 + bm_h - depth;
    let cx = bm_x + bm_w / 2;
    for yy in apex_y..y0 + bm_h {
        let half = (bm_w / 2) * (yy - apex_y) / (depth - 1);
        fill_rect(buf, cx - half, yy, half * 2, 1, BG);
    }
}

fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    table
}

fn crc32_feed(crc: u32, data: &[u8], table: &[u32; 256]) -> u32 {
    let mut c = crc;
    for &b in data {
        c = table[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8);
    }
    c
}

fn chunk_crc(ctype: &[u8; 4], data: &[u8]) -> u32 {
    let table = crc32_table();
    let c = crc32_feed(0xFFFF_FFFF, ctype, &table);
    let c = crc32_feed(c, data, &table);
    c ^ 0xFFFF_FFFF
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u64 = 65521;
    let mut a: u64 = 1;
    let mut b: u64 = 0;
    for block in data.chunks(5552) {
        for &x in block {
            a += u64::from(x);
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    ((b as u32) << 16) | a as u32
}

fn push_chunk(out: &mut Vec<u8>, ctype: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(ctype);
    out.extend_from_slice(data);
    out.extend_from_slice(&chunk_crc(ctype, data).to_be_bytes());
}

fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut z = Vec::with_capacity(data.len() + data.len() / 65535 * 5 + 6);
    z.extend_from_slice(&[0x78, 0x01]); // deflate, 32K window, no preset dict
    let mut off = 0;
    loop {
        let len = (data.len() - off).min(65535);
        let last = off + len == data.len();
        z.push(if last { 1 } else { 0 }); // BFINAL + BTYPE=stored
        let l = len as u16;
        z.extend_from_slice(&l.to_le_bytes()); // LEN
        z.extend_from_slice(&(!l).to_le_bytes()); // NLEN (one's complement)
        z.extend_from_slice(&data[off..off + len]);
        off += len;
        if last {
            break;
        }
    }
    z.extend_from_slice(&adler32(data).to_be_bytes());
    z
}

fn encode_png(rgb: &[u8], width: u32, height: u32) -> Vec<u8> {
    let stride = width as usize * 3;
    let mut raw = Vec::with_capacity(height as usize * (stride + 1));
    for y in 0..height as usize {
        raw.push(0); // filter type: None
        raw.extend_from_slice(&rgb[y * stride..(y + 1) * stride]);
    }

    let mut out = Vec::with_capacity(raw.len() + raw.len() / 65535 * 5 + 64);
    out.extend_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8; // bit depth
    ihdr[9] = 2; // color type 2: truecolor RGB
    ihdr[10] = 0; // compression method: zlib/deflate
    ihdr[11] = 0; // filter method
    ihdr[12] = 0; // interlace method: none
    push_chunk(&mut out, b"IHDR", &ihdr);
    push_chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    push_chunk(&mut out, b"IEND", &[]);
    out
}

fn output_path() -> PathBuf {
    if let Some(arg) = env::args().nth(1) {
        return PathBuf::from(arg);
    }
    let manifest = env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .or_else(|| option_env!("CARGO_MANIFEST_DIR").map(PathBuf::from));
    match manifest {
        Some(dir) => dir.join("icons").join("src.png"),
        None => PathBuf::from("../icons/src.png"),
    }
}

fn main() {
    let out = output_path();
    let mut buf = vec![0u8; W * H * 3];
    draw(&mut buf);
    let png = encode_png(&buf, W as u32, H as u32);
    if let Some(dir) = out.parent() {
        let _ = fs::create_dir_all(dir);
    }
    fs::write(&out, &png).unwrap_or_else(|e| panic!("failed to write {}: {e}", out.display()));
    println!("wrote {} ({} bytes)", out.display(), png.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc(data: &[u8]) -> u32 {
        let table = crc32_table();
        crc32_feed(0xFFFF_FFFF, data, &table) ^ 0xFFFF_FFFF
    }

    #[test]
    fn crc32_known_answers() {
        assert_eq!(crc(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc(b"IEND"), 0xAE42_6082);
    }

    #[test]
    fn adler32_known_answer() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn encodes_minimal_png() {
        let png = encode_png(&[1, 2, 3], 1, 1);
        assert_eq!(png.len(), 72);
        assert_eq!(&png[0..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A][..]);
        assert_eq!(&png[8..12], &13u32.to_be_bytes()[..]);
        assert_eq!(&png[12..16], &b"IHDR"[..]);
        assert_eq!(&png[16..24], &[0, 0, 0, 1, 0, 0, 0, 1][..]);
        assert_eq!(&png[24..29], &[8, 2, 0, 0, 0][..]);
        assert_eq!(&png[33..37], &15u32.to_be_bytes()[..]);
        assert_eq!(&png[37..41], &b"IDAT"[..]);
        assert_eq!(&png[41..43], &[0x78, 0x01][..]);
        assert_eq!(png[43], 0x01);
        assert_eq!(&png[44..48], &[0x04, 0x00, 0xFB, 0xFF][..]);
        assert_eq!(&png[48..52], &[0x00, 0x01, 0x02, 0x03][..]);
        assert_eq!(&png[52..56], &[0x00, 0x0E, 0x00, 0x07][..]);
        assert_eq!(&png[64..68], &b"IEND"[..]);
        assert_eq!(&png[68..72], &0xAE42_6082u32.to_be_bytes()[..]);
    }
}
