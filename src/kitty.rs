use std::{
    io::{self, Write},
    thread,
    time::Duration,
};

use base64::{Engine, prelude::BASE64_STANDARD};
use flate2::{Compression, write::ZlibEncoder};

const KITTY_CHUNK_BYTES: usize = 4096;

pub fn write_rgb_frame<W: Write>(
    writer: &mut W,
    pixels: &[u8],
    width_px: u32,
    height_px: u32,
    cols: u32,
    rows: u32,
    prevent_cursor_move: bool,
) -> io::Result<()> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(pixels)?;
    let compressed = encoder.finish()?;
    let encoded = BASE64_STANDARD.encode(&compressed);
    let bytes = encoded.as_bytes();
    let mut offset = 0;

    while offset < bytes.len() {
        let end = (offset + KITTY_CHUNK_BYTES).min(bytes.len());
        let more = i32::from(end != bytes.len());
        let mut packet = Vec::new();

        if offset == 0 {
            let cursor_policy = if prevent_cursor_move { ",C=1" } else { "" };
            write!(
                packet,
                "\x1b_Ga=T,f=24,o=z,s={},v={},c={},r={}{},q=2,m={};",
                width_px,
                height_px,
                cols.max(1),
                rows.max(1),
                cursor_policy,
                more
            )?;
        } else {
            write!(packet, "\x1b_Gq=2,m={};", more)?;
        }

        packet.write_all(&bytes[offset..end])?;
        packet.write_all(b"\x1b\\")?;
        write_all_robust(writer, &packet)?;
        offset = end;
    }

    Ok(())
}

pub fn write_all_robust<W: Write>(writer: &mut W, mut buffer: &[u8]) -> io::Result<()> {
    while !buffer.is_empty() {
        match writer.write(buffer) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to write whole buffer",
                ));
            }
            Ok(bytes) => buffer = &buffer[bytes..],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) if error.raw_os_error() == Some(35) => {
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub fn flush_robust<W: Write>(writer: &mut W) -> io::Result<()> {
    loop {
        match writer.flush() {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) if error.raw_os_error() == Some(35) => {
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_compressed_rgb_kitty_frame() {
        let pixels = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        let mut out = Vec::new();

        write_rgb_frame(&mut out, &pixels, 2, 2, 8, 4, true).unwrap();
        let text = String::from_utf8_lossy(&out);

        assert!(text.contains("\u{1b}_Ga=T"));
        assert!(text.contains("f=24"));
        assert!(text.contains("o=z"));
        assert!(text.contains("s=2"));
        assert!(text.contains("v=2"));
        assert!(text.contains("c=8"));
        assert!(text.contains("r=4"));
        assert!(text.contains("C=1"));
        assert!(text.contains("q=2"));
    }

    #[test]
    fn chunks_large_frames() {
        let mut pixels = Vec::with_capacity(256 * 256 * 3);
        for i in 0..(256 * 256) {
            pixels.push((i & 0xff) as u8);
            pixels.push(((i >> 8) & 0xff) as u8);
            pixels.push(((i >> 16) & 0xff) as u8);
        }

        let mut out = Vec::new();
        write_rgb_frame(&mut out, &pixels, 256, 256, 80, 40, true).unwrap();
        let text = String::from_utf8_lossy(&out);
        let chunks: Vec<&str> = text.split("\u{1b}_G").skip(1).collect();

        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|chunk| chunk.contains("q=2")));
        assert!(chunks.first().unwrap().contains("m=1"));
        assert!(chunks.last().unwrap().contains("m=0"));
    }
}
