//! Pure Safe Rust Synthetic PNG Canvas Generator.
//!
//! Provides an authentic, self-contained 8-bit RGBA pixel canvas and standard PNG encoder
//! compliant with ISO/IEC 15948 / RFC 2083 and RFC 1951 (Deflate uncompressed blocks).
//! Zero external browser or image crate dependencies.
//! Strict `#![forbid(unsafe_code)]` compliance.

use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use sha2::{Digest, Sha256};

/// Standard PNG 8-byte magic file signature.
pub const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// Precomputed ISO 3309 / IEEE 802.3 CRC-32 lookup table (polynomial 0xEDB88320).
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0usize;
        while k < 8 {
            if (c & 1) != 0 {
                c = 0xEDB88320 ^ (c >> 1);
            } else {
                c >>= 1;
            }
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

/// Computes the IEEE 802.3 CRC-32 checksum for the provided byte slice.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        let idx = ((crc ^ (b as u32)) & 0xFF) as usize;
        crc = CRC32_TABLE[idx] ^ (crc >> 8);
    }
    !crc
}

/// Computes the RFC 1950 Adler-32 checksum for the provided byte slice.
pub fn adler32(data: &[u8]) -> u32 {
    let mut s1 = 1u32;
    let mut s2 = 0u32;
    const MOD_ADLER: u32 = 65521;

    for &b in data {
        s1 = (s1 + b as u32) % MOD_ADLER;
        s2 = (s2 + s1) % MOD_ADLER;
    }
    (s2 << 16) | s1
}

/// Compact 5x7 bitmap font covering printable ASCII 32 (' ') through 126 ('~').
/// Each character is represented by 5 column bytes, where bits 0..6 represent row pixels.
static FONT_5X7: [[u8; 5]; 95] = [
    [0x00, 0x00, 0x00, 0x00, 0x00], // 32 ' '
    [0x00, 0x00, 0x5F, 0x00, 0x00], // 33 '!'
    [0x00, 0x07, 0x00, 0x07, 0x00], // 34 '"'
    [0x14, 0x7F, 0x14, 0x7F, 0x14], // 35 '#'
    [0x24, 0x2A, 0x7F, 0x2A, 0x12], // 36 '$'
    [0x23, 0x13, 0x08, 0x64, 0x62], // 37 '%'
    [0x36, 0x49, 0x55, 0x22, 0x50], // 38 '&'
    [0x00, 0x05, 0x03, 0x00, 0x00], // 39 '\''
    [0x00, 0x1C, 0x22, 0x41, 0x00], // 40 '('
    [0x00, 0x41, 0x22, 0x1C, 0x00], // 41 ')'
    [0x14, 0x08, 0x3E, 0x08, 0x14], // 42 '*'
    [0x08, 0x08, 0x3E, 0x08, 0x08], // 43 '+'
    [0x00, 0x50, 0x30, 0x00, 0x00], // 44 ','
    [0x08, 0x08, 0x08, 0x08, 0x08], // 45 '-'
    [0x00, 0x60, 0x60, 0x00, 0x00], // 46 '.'
    [0x20, 0x10, 0x08, 0x04, 0x02], // 47 '/'
    [0x3E, 0x51, 0x49, 0x45, 0x3E], // 48 '0'
    [0x00, 0x42, 0x7F, 0x40, 0x00], // 49 '1'
    [0x42, 0x61, 0x51, 0x49, 0x46], // 50 '2'
    [0x21, 0x41, 0x45, 0x4B, 0x31], // 51 '3'
    [0x18, 0x14, 0x12, 0x7F, 0x10], // 52 '4'
    [0x27, 0x45, 0x45, 0x45, 0x39], // 53 '5'
    [0x3C, 0x4A, 0x49, 0x49, 0x30], // 54 '6'
    [0x01, 0x71, 0x09, 0x05, 0x03], // 55 '7'
    [0x36, 0x49, 0x49, 0x49, 0x36], // 56 '8'
    [0x06, 0x49, 0x49, 0x29, 0x1E], // 57 '9'
    [0x00, 0x36, 0x36, 0x00, 0x00], // 58 ':'
    [0x00, 0x56, 0x36, 0x00, 0x00], // 59 ';'
    [0x08, 0x14, 0x22, 0x41, 0x00], // 60 '<'
    [0x14, 0x14, 0x14, 0x14, 0x14], // 61 '='
    [0x00, 0x41, 0x22, 0x14, 0x08], // 62 '>'
    [0x02, 0x01, 0x51, 0x09, 0x06], // 63 '?'
    [0x32, 0x49, 0x79, 0x41, 0x3E], // 64 '@'
    [0x7E, 0x11, 0x11, 0x11, 0x7E], // 65 'A'
    [0x7F, 0x49, 0x49, 0x49, 0x36], // 66 'B'
    [0x3E, 0x41, 0x41, 0x41, 0x22], // 67 'C'
    [0x7F, 0x41, 0x41, 0x22, 0x1C], // 68 'D'
    [0x7F, 0x49, 0x49, 0x49, 0x41], // 69 'E'
    [0x7F, 0x09, 0x09, 0x09, 0x01], // 70 'F'
    [0x3E, 0x41, 0x49, 0x49, 0x7A], // 71 'G'
    [0x7F, 0x08, 0x08, 0x08, 0x7F], // 72 'H'
    [0x00, 0x41, 0x7F, 0x41, 0x00], // 73 'I'
    [0x20, 0x40, 0x41, 0x3F, 0x01], // 74 'J'
    [0x7F, 0x08, 0x14, 0x22, 0x41], // 75 'K'
    [0x7F, 0x40, 0x40, 0x40, 0x40], // 76 'L'
    [0x7F, 0x02, 0x0C, 0x02, 0x7F], // 77 'M'
    [0x7F, 0x04, 0x08, 0x10, 0x7F], // 78 'N'
    [0x3E, 0x41, 0x41, 0x41, 0x3E], // 79 'O'
    [0x7F, 0x09, 0x09, 0x09, 0x06], // 80 'P'
    [0x3E, 0x41, 0x51, 0x21, 0x5E], // 81 'Q'
    [0x7F, 0x09, 0x19, 0x29, 0x46], // 82 'R'
    [0x46, 0x49, 0x49, 0x49, 0x31], // 83 'S'
    [0x01, 0x01, 0x7F, 0x01, 0x01], // 84 'T'
    [0x3F, 0x40, 0x40, 0x40, 0x3F], // 85 'U'
    [0x1F, 0x20, 0x40, 0x20, 0x1F], // 86 'V'
    [0x3F, 0x40, 0x38, 0x40, 0x3F], // 87 'W'
    [0x63, 0x14, 0x08, 0x14, 0x63], // 88 'X'
    [0x07, 0x08, 0x70, 0x08, 0x07], // 89 'Y'
    [0x61, 0x51, 0x49, 0x45, 0x43], // 90 'Z'
    [0x00, 0x7F, 0x41, 0x41, 0x00], // 91 '['
    [0x02, 0x04, 0x08, 0x10, 0x20], // 92 '\'
    [0x00, 0x41, 0x41, 0x7F, 0x00], // 93 ']'
    [0x04, 0x02, 0x01, 0x02, 0x04], // 94 '^'
    [0x40, 0x40, 0x40, 0x40, 0x40], // 95 '_'
    [0x00, 0x01, 0x02, 0x04, 0x00], // 96 '`'
    [0x20, 0x54, 0x54, 0x54, 0x78], // 97 'a'
    [0x7F, 0x48, 0x44, 0x44, 0x38], // 98 'b'
    [0x38, 0x44, 0x44, 0x44, 0x20], // 99 'c'
    [0x38, 0x44, 0x44, 0x48, 0x7F], // 100 'd'
    [0x38, 0x54, 0x54, 0x54, 0x18], // 101 'e'
    [0x08, 0x7E, 0x09, 0x01, 0x02], // 102 'f'
    [0x08, 0x14, 0x54, 0x54, 0x3C], // 103 'g'
    [0x7F, 0x08, 0x04, 0x04, 0x78], // 104 'h'
    [0x00, 0x44, 0x7D, 0x40, 0x00], // 105 'i'
    [0x20, 0x40, 0x44, 0x3D, 0x00], // 106 'j'
    [0x7F, 0x10, 0x28, 0x44, 0x00], // 107 'k'
    [0x00, 0x41, 0x7F, 0x40, 0x00], // 108 'l'
    [0x7C, 0x04, 0x18, 0x04, 0x78], // 109 'm'
    [0x7C, 0x08, 0x04, 0x04, 0x78], // 110 'n'
    [0x38, 0x44, 0x44, 0x44, 0x38], // 111 'o'
    [0x7C, 0x14, 0x14, 0x14, 0x08], // 112 'p'
    [0x08, 0x14, 0x14, 0x18, 0x7C], // 113 'q'
    [0x7C, 0x08, 0x04, 0x04, 0x08], // 114 'r'
    [0x48, 0x54, 0x54, 0x54, 0x20], // 115 's'
    [0x04, 0x3F, 0x44, 0x40, 0x20], // 116 't'
    [0x3C, 0x40, 0x40, 0x20, 0x7C], // 117 'u'
    [0x1C, 0x20, 0x40, 0x20, 0x1C], // 118 'v'
    [0x3C, 0x40, 0x30, 0x40, 0x3C], // 119 'w'
    [0x44, 0x28, 0x10, 0x28, 0x44], // 120 'x'
    [0x0C, 0x50, 0x50, 0x50, 0x3C], // 121 'y'
    [0x44, 0x64, 0x54, 0x4C, 0x44], // 122 'z'
    [0x00, 0x08, 0x36, 0x41, 0x00], // 123 '{'
    [0x00, 0x00, 0x7F, 0x00, 0x00], // 124 '|'
    [0x00, 0x41, 0x36, 0x08, 0x00], // 125 '}'
    [0x08, 0x08, 0x2A, 0x18, 0x08], // 126 '~'
];

/// Pure safe Rust synthetic pixel canvas for rasterizing layouts and generating valid PNGs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntheticCanvas {
    width: u32,
    height: u32,
    pixels: Vec<u8>, // RGBA 4 bytes per pixel
}

impl SyntheticCanvas {
    /// Creates a new canvas of the given dimensions with transparent black pixels.
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        Self {
            width,
            height,
            pixels: vec![0u8; size],
        }
    }

    /// Creates a new canvas initialized to an opaque white background.
    pub fn new_white(width: u32, height: u32) -> Self {
        let mut canvas = Self::new(width, height);
        canvas.fill(255, 255, 255, 255);
        canvas
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Fills the entire canvas with a uniform RGBA color.
    pub fn fill(&mut self, r: u8, g: u8, b: u8, a: u8) {
        let mut i = 0;
        while i + 3 < self.pixels.len() {
            self.pixels[i] = r;
            self.pixels[i + 1] = g;
            self.pixels[i + 2] = b;
            self.pixels[i + 3] = a;
            i += 4;
        }
    }

    /// Sets a single pixel color with bounds checking.
    pub fn set_pixel(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
        if x < self.width && y < self.height {
            let offset = ((y * self.width + x) * 4) as usize;
            if offset + 3 < self.pixels.len() {
                self.pixels[offset] = r;
                self.pixels[offset + 1] = g;
                self.pixels[offset + 2] = b;
                self.pixels[offset + 3] = a;
            }
        }
    }

    /// Retrieves the RGBA pixel value at coordinates (x, y). Returns transparent if out of bounds.
    pub fn get_pixel(&self, x: u32, y: u32) -> (u8, u8, u8, u8) {
        if x < self.width && y < self.height {
            let offset = ((y * self.width + x) * 4) as usize;
            if offset + 3 < self.pixels.len() {
                return (
                    self.pixels[offset],
                    self.pixels[offset + 1],
                    self.pixels[offset + 2],
                    self.pixels[offset + 3],
                );
            }
        }
        (0, 0, 0, 0)
    }

    /// Draws a filled rectangle with an optional border.
    pub fn draw_rect(
        &mut self,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        fill: Option<(u8, u8, u8, u8)>,
        border: Option<((u8, u8, u8, u8), u32)>,
    ) {
        if w == 0 || h == 0 {
            return;
        }

        // 1. Fill interior if specified
        if let Some((r, g, b, a)) = fill {
            let x_end = (x + w).min(self.width);
            let y_end = (y + h).min(self.height);
            for py in y..y_end {
                for px in x..x_end {
                    self.set_pixel(px, py, r, g, b, a);
                }
            }
        }

        // 2. Draw border if specified
        if let Some(((br, bg, bb, ba), thickness)) = border
            && thickness > 0
        {
            let x_end = (x + w).min(self.width);
            let y_end = (y + h).min(self.height);

            for t in 0..thickness {
                // Top and bottom borders
                let top_y = y + t;
                let bottom_y = if y + h > t { y + h - 1 - t } else { y };
                for px in x..x_end {
                    if top_y < self.height {
                        self.set_pixel(px, top_y, br, bg, bb, ba);
                    }
                    if bottom_y < self.height {
                        self.set_pixel(px, bottom_y, br, bg, bb, ba);
                    }
                }

                // Left and right borders
                let left_x = x + t;
                let right_x = if x + w > t { x + w - 1 - t } else { x };
                for py in y..y_end {
                    if left_x < self.width {
                        self.set_pixel(left_x, py, br, bg, bb, ba);
                    }
                    if right_x < self.width {
                        self.set_pixel(right_x, py, br, bg, bb, ba);
                    }
                }
            }
        }
    }

    /// Renders text starting at (x, y) with the built-in 5x7 font.
    /// `scale` multiplies the glyph dimensions (1 = 5x7, 2 = 10x14, etc.).
    pub fn draw_text(&mut self, x: u32, y: u32, text: &str, color: (u8, u8, u8, u8), scale: u32) {
        let (r, g, b, a) = color;
        let scale = scale.max(1);
        let mut cur_x = x;

        for ch in text.chars() {
            if ch == '\n' {
                cur_x = x;
                // Move down font height (7 * scale) + line gap (2 * scale)
                continue;
            }

            let code = ch as usize;
            if (32..=126).contains(&code) {
                let glyph = &FONT_5X7[code - 32];
                for (col_idx, &col_byte) in glyph.iter().enumerate() {
                    for row_idx in 0..7 {
                        if (col_byte & (1 << row_idx)) != 0 {
                            let px_start = cur_x + (col_idx as u32) * scale;
                            let py_start = y + (row_idx as u32) * scale;
                            for sy in 0..scale {
                                for sx in 0..scale {
                                    self.set_pixel(px_start + sx, py_start + sy, r, g, b, a);
                                }
                            }
                        }
                    }
                }
            }
            // Advance character spacing: 5 cols + 1 col spacing = 6 * scale
            cur_x += 6 * scale;
            if cur_x >= self.width {
                break;
            }
        }
    }

    /// Encodes canvas pixels into standard PNG byte stream (8-bit RGBA).
    ///
    /// Implements RFC 2083 standard chunks:
    /// - `IHDR`: 13 bytes metadata (width, height, 8-bit depth, RGBA color type 6, no interlace)
    /// - `IDAT`: Uncompressed Deflate blocks (RFC 1951 §3.2.4) wrapped in Zlib header with Adler-32
    /// - `IEND`: End of image marker
    /// - Valid ISO 3309 CRC-32 on every chunk
    pub fn to_png_bytes(&self) -> Vec<u8> {
        let mut png = Vec::with_capacity(PNG_SIGNATURE.len() + 256 + self.pixels.len());

        // 1. Write PNG 8-byte signature
        png.extend_from_slice(&PNG_SIGNATURE);

        // 2. Write IHDR chunk
        let mut ihdr_data = Vec::with_capacity(13);
        ihdr_data.extend_from_slice(&self.width.to_be_bytes());
        ihdr_data.extend_from_slice(&self.height.to_be_bytes());
        ihdr_data.push(8); // Bit depth: 8
        ihdr_data.push(6); // Color type: 6 (RGBA)
        ihdr_data.push(0); // Compression method: 0 (deflate)
        ihdr_data.push(0); // Filter method: 0 (standard adaptive filter)
        ihdr_data.push(0); // Interlace method: 0 (no interlace)
        write_chunk(&mut png, b"IHDR", &ihdr_data);

        // 3. Assemble scanlines: 1 filter byte (0 = None) per row, followed by row RGBA pixels
        let row_bytes = (self.width as usize) * 4;
        let uncompressed_len = (self.height as usize) * (1 + row_bytes);
        let mut raw_scanlines = Vec::with_capacity(uncompressed_len);

        for row in 0..self.height {
            raw_scanlines.push(0u8); // Filter type 0: None
            let row_start = (row as usize) * row_bytes;
            let row_end = row_start + row_bytes;
            if row_end <= self.pixels.len() {
                raw_scanlines.extend_from_slice(&self.pixels[row_start..row_end]);
            } else {
                // Safeguard against short pixel buffer
                raw_scanlines.resize(raw_scanlines.len() + row_bytes, 0u8);
            }
        }

        // 4. Compute Adler-32 over raw scanlines
        let adler = adler32(&raw_scanlines);

        // 5. Build Zlib-wrapped Deflate stream containing uncompressed blocks (RFC 1951 §3.2.4)
        // Max uncompressed block size is 65535 (0xFFFF) bytes.
        let mut zlib_stream = Vec::with_capacity(raw_scanlines.len() + 64);
        zlib_stream.push(0x78); // CMF: Deflate, 32K window
        zlib_stream.push(0x01); // FLG: FCHECK set so (0x7800 + 0x01) % 31 == 0

        const MAX_BLOCK_LEN: usize = 65535;
        let chunks: Vec<&[u8]> = raw_scanlines.chunks(MAX_BLOCK_LEN).collect();
        if chunks.is_empty() {
            // Empty image fallback: 1 empty final block
            zlib_stream.push(0x01); // BFINAL=1, BTYPE=00
            zlib_stream.extend_from_slice(&0u16.to_le_bytes());
            zlib_stream.extend_from_slice(&0xFFFFu16.to_le_bytes());
        } else {
            for (idx, chunk) in chunks.iter().enumerate() {
                let is_last = idx == chunks.len() - 1;
                let bfinal_btype = if is_last { 0x01u8 } else { 0x00u8 };
                zlib_stream.push(bfinal_btype);

                let len = chunk.len() as u16;
                let nlen = !len;
                zlib_stream.extend_from_slice(&len.to_le_bytes());
                zlib_stream.extend_from_slice(&nlen.to_le_bytes());
                zlib_stream.extend_from_slice(chunk);
            }
        }

        // Append 4-byte Adler-32 checksum (Big Endian)
        zlib_stream.extend_from_slice(&adler.to_be_bytes());

        // Write IDAT chunk
        write_chunk(&mut png, b"IDAT", &zlib_stream);

        // 6. Write IEND chunk (0 length)
        write_chunk(&mut png, b"IEND", &[]);

        png
    }

    /// Converts the canvas to a Base64 encoded PNG string.
    pub fn to_base64_png(&self) -> String {
        let bytes = self.to_png_bytes();
        BASE64_STANDARD.encode(&bytes)
    }

    /// Computes the SHA-256 hex digest of the encoded PNG bytes for deterministic verification.
    pub fn sha256_digest(&self) -> String {
        let bytes = self.to_png_bytes();
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        format!("{:x}", hasher.finalize())
    }
}

/// Writes a standard PNG chunk (Length + Type + Data + CRC-32) into the output vector.
fn write_chunk(output: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    let len = data.len() as u32;
    output.extend_from_slice(&len.to_be_bytes());
    output.extend_from_slice(chunk_type);
    output.extend_from_slice(data);

    // CRC is computed over Chunk Type + Chunk Data
    let mut crc_buf = Vec::with_capacity(4 + data.len());
    crc_buf.extend_from_slice(chunk_type);
    crc_buf.extend_from_slice(data);
    let checksum = crc32(&crc_buf);
    output.extend_from_slice(&checksum.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_standard_vector() {
        // Standard test vector: "123456789" -> 0xCBF43926
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
    }

    #[test]
    fn adler32_standard_vector() {
        // Standard test vector: "123456789" -> 0x091E01DE
        assert_eq!(adler32(b"123456789"), 0x091E01DE);
    }

    #[test]
    fn canvas_basic_dimensions_and_pixels() {
        let mut canvas = SyntheticCanvas::new(20, 10);
        assert_eq!(canvas.width(), 20);
        assert_eq!(canvas.height(), 10);
        assert_eq!(canvas.pixels().len(), 20 * 10 * 4);

        canvas.set_pixel(5, 5, 255, 0, 128, 255);
        assert_eq!(canvas.get_pixel(5, 5), (255, 0, 128, 255));
        assert_eq!(canvas.get_pixel(0, 0), (0, 0, 0, 0));
        assert_eq!(canvas.get_pixel(100, 100), (0, 0, 0, 0)); // Out of bounds
    }

    #[test]
    fn canvas_draw_rect_and_text() {
        let mut canvas = SyntheticCanvas::new_white(100, 50);
        assert_eq!(canvas.get_pixel(10, 10), (255, 255, 255, 255));

        // Draw blue rectangle with black border
        canvas.draw_rect(
            10,
            10,
            30,
            20,
            Some((0, 0, 255, 255)),
            Some(((0, 0, 0, 255), 1)),
        );

        // Check border pixel
        assert_eq!(canvas.get_pixel(10, 10), (0, 0, 0, 255));
        // Check interior pixel
        assert_eq!(canvas.get_pixel(15, 15), (0, 0, 255, 255));

        // Draw text
        canvas.draw_text(50, 10, "Hi", (255, 0, 0, 255), 1);
        let mut found_red = false;
        for py in 10..20 {
            for px in 50..70 {
                if canvas.get_pixel(px, py) == (255, 0, 0, 255) {
                    found_red = true;
                    break;
                }
            }
        }
        assert!(found_red, "draw_text must produce non-empty pixel glyphs");
    }

    #[test]
    fn png_structure_validity() {
        let mut canvas = SyntheticCanvas::new(16, 16);
        canvas.fill(100, 150, 200, 255);

        let png = canvas.to_png_bytes();
        assert!(png.len() > 64);
        assert_eq!(&png[0..8], &PNG_SIGNATURE);

        // Verify IHDR chunk exists at index 8
        let ihdr_len = u32::from_be_bytes([png[8], png[9], png[10], png[11]]);
        assert_eq!(ihdr_len, 13);
        assert_eq!(&png[12..16], b"IHDR");

        // Verify width and height inside IHDR
        let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
        let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
        assert_eq!(w, 16);
        assert_eq!(h, 16);

        // Verify Base64 and SHA-256 digest
        let b64 = canvas.to_base64_png();
        assert!(!b64.is_empty());
        let digest = canvas.sha256_digest();
        assert_eq!(digest.len(), 64);
    }
}
