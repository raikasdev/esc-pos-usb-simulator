//! Decodes a raw ESC/POS byte stream into structured print events.
//!
//! Matches the command set actually emitted by
//! `@point-of-sale/receipt-printer-encoder` (the `esc-pos` language, as used
//! by the WebUSBReceiptPrinter library): ESC @ / FS . (init), ESC M (font),
//! ESC a (align), ESC E (bold), ESC - (underline), ESC 4 (italic), GS B
//! (invert), GS ! (size), ESC t (codepage select — see codepage.rs),
//! GS v 0 (raster image), GS ( k (QR code / PDF417), GS k (1D barcode),
//! ESC p (cash drawer pulse), GS V (cut), plus plain text and line feeds.

use base64::Engine;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Init,
    Align { value: &'static str },
    Style { bold: bool, italic: bool, underline: bool, invert: bool, width: u8, height: u8 },
    Text { text: String },
    NewLine,
    Cut,
    Image { width: u32, height: u32, bits: String },
    QrCode { data: String },
    Barcode { symbology: &'static str, data: String },
    Pulse,
}

/// Accumulates `ESC *` column-mode image strips (each up to 24 pixel rows
/// tall, spanning the full image width) into the same row-major 1bpp layout
/// `GS v 0` uses, so both paths feed the same `Event::Image`.
struct ColumnImage {
    width_bytes: u32,
    rows: Vec<u8>,
}

pub struct Parser {
    buf: Vec<u8>,
    bold: bool,
    italic: bool,
    underline: bool,
    invert: bool,
    width: u8,
    height: u8,
    codepage: crate::codepage::Codepage,
    pending_qr_data: Option<String>,
    column_image: Option<ColumnImage>,
}

impl Parser {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            bold: false,
            italic: false,
            underline: false,
            invert: false,
            width: 1,
            height: 1,
            codepage: crate::codepage::Codepage::Cp437,
            pending_qr_data: None,
            column_image: None,
        }
    }

    fn style_event(&self) -> Event {
        Event::Style {
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            invert: self.invert,
            width: self.width,
            height: self.height,
        }
    }

    fn reset_style(&mut self) {
        self.bold = false;
        self.italic = false;
        self.underline = false;
        self.invert = false;
        self.width = 1;
        self.height = 1;
        self.codepage = crate::codepage::Codepage::Cp437;
    }

    /// Feed newly-received bytes and return every event they complete.
    /// Bytes that form an incomplete command are held until more data arrives.
    pub fn feed(&mut self, data: &[u8]) -> Vec<Event> {
        self.buf.extend_from_slice(data);
        let mut events = Vec::new();
        let mut i = 0;

        while i < self.buf.len() {
            match self.buf[i] {
                b'\n' => {
                    events.push(Event::NewLine);
                    i += 1;
                }
                b'\r' => {
                    i += 1;
                }
                0x1b => match self.parse_esc(i, &mut events) {
                    Some(consumed) => i += consumed,
                    None => break,
                },
                0x1d => match self.parse_gs(i, &mut events) {
                    Some(consumed) => i += consumed,
                    None => break,
                },
                0x1c => match self.parse_fs(i) {
                    Some(consumed) => i += consumed,
                    None => break,
                },
                _ => {
                    let start = i;
                    while i < self.buf.len() && !matches!(self.buf[i], 0x1b | 0x1d | 0x1c | b'\n' | b'\r') {
                        i += 1;
                    }
                    let text = crate::codepage::decode(self.codepage, &self.buf[start..i]);
                    if !text.is_empty() {
                        events.push(Event::Text { text });
                    }
                }
            }
        }

        self.buf.drain(0..i);
        events
    }

    /// `FS .` — select single-byte character mode. No visible effect for us.
    fn parse_fs(&self, i: usize) -> Option<usize> {
        self.buf.get(i + 1)?;
        Some(2)
    }

    fn parse_esc(&mut self, i: usize, events: &mut Vec<Event>) -> Option<usize> {
        let data = &self.buf[i..];
        match *data.get(1)? {
            b'@' => {
                self.reset_style();
                events.push(Event::Init);
                Some(2)
            }
            b'M' => {
                data.get(2)?;
                Some(3) // font select — not rendered differently for now
            }
            b'a' => {
                let n = *data.get(2)?;
                let value = match n {
                    1 => "center",
                    2 => "right",
                    _ => "left",
                };
                events.push(Event::Align { value });
                Some(3)
            }
            b'E' => {
                let n = *data.get(2)?;
                self.bold = n != 0;
                events.push(self.style_event());
                Some(3)
            }
            b'-' => {
                let n = *data.get(2)?;
                self.underline = n != 0;
                events.push(self.style_event());
                Some(3)
            }
            b'4' => {
                let n = *data.get(2)?;
                self.italic = n != 0;
                events.push(self.style_event());
                Some(3)
            }
            b't' => {
                let n = *data.get(2)?;
                self.codepage = crate::codepage::codepage_from_star_index(n);
                Some(3)
            }
            b'p' => {
                data.get(4)?; // device, t1, t2
                events.push(Event::Pulse);
                Some(5)
            }
            b'3' => {
                data.get(2)?; // line spacing (column-mode image prep)
                Some(3)
            }
            b'2' => {
                // Reset line spacing — also how the encoder marks the end
                // of a column-mode image; flush whatever we accumulated.
                if let Some(image) = self.column_image.take() {
                    events.push(Event::Image {
                        width: image.width_bytes * 8,
                        height: (image.rows.len() as u32) / image.width_bytes,
                        bits: base64::engine::general_purpose::STANDARD.encode(&image.rows),
                    });
                }
                Some(2)
            }
            b'*' => self.parse_column_image_strip(i),
            b'!' => {
                // Combined print-mode byte (bold + double width/height) —
                // some simpler client libraries send this instead of the
                // separate ESC E / GS ! commands; support both.
                let n = *data.get(2)?;
                self.bold = n & 0x08 != 0;
                self.width = if n & 0x20 != 0 { 2 } else { 1 };
                self.height = if n & 0x10 != 0 { 2 } else { 1 };
                events.push(self.style_event());
                Some(3)
            }
            _ => Some(2),
        }
    }

    fn parse_gs(&mut self, i: usize, events: &mut Vec<Event>) -> Option<usize> {
        let data = &self.buf[i..];
        match *data.get(1)? {
            b'V' => {
                let m = *data.get(2)?;
                let consumed = if matches!(m, 65 | 66) {
                    data.get(3)?;
                    4
                } else {
                    3
                };
                events.push(Event::Cut);
                Some(consumed)
            }
            b'!' => {
                let n = *data.get(2)?;
                self.height = (n & 0x0f) + 1;
                self.width = ((n >> 4) & 0x0f) + 1;
                events.push(self.style_event());
                Some(3)
            }
            b'B' => {
                let n = *data.get(2)?;
                self.invert = n != 0;
                events.push(self.style_event());
                Some(3)
            }
            b'h' | b'w' | b'H' => {
                data.get(2)?; // barcode height / width / text-position
                Some(3)
            }
            b'k' => self.parse_barcode(i, events),
            b'v' => self.parse_raster_image(i, events),
            b'(' if data.get(2) == Some(&b'k') => self.parse_2d_symbol(i, events),
            _ => Some(2),
        }
    }

    /// `ESC * m nL nH data... LF` — one column-mode image strip, up to 24
    /// pixel rows tall and `width` columns wide. `m` selects 8 dots/column
    /// (1 byte) or 24 dots/column (3 bytes); this encoder always uses the
    /// latter. Bits are column-major (bit 7 of byte 0 is the topmost pixel
    /// of that column) — converted here into the row-major layout
    /// `Event::Image` expects, and accumulated across strips since a tall
    /// image arrives as multiple consecutive `ESC *` calls.
    fn parse_column_image_strip(&mut self, i: usize) -> Option<usize> {
        let width;
        let bytes_per_col;
        let strip: Vec<u8>;
        {
            let data = &self.buf[i..];
            let m = *data.get(2)?;
            let n_l = *data.get(3)? as u32;
            let n_h = *data.get(4)? as u32;
            width = n_l + n_h * 256;
            bytes_per_col = if matches!(m, 32 | 33) { 3 } else { 1 };
            let total = (width * bytes_per_col) as usize;
            let columns = data.get(5..5 + total)?;
            data.get(5 + total)?; // trailing LF
            strip = columns.to_vec();
        }

        let width_bytes = width.div_ceil(8);
        let dots_per_col = bytes_per_col * 8;
        let image = self.column_image.get_or_insert_with(|| ColumnImage { width_bytes, rows: Vec::new() });

        let row_offset = (image.rows.len() as u32) / width_bytes;
        image.rows.resize(image.rows.len() + (width_bytes * dots_per_col) as usize, 0);

        for x in 0..width {
            for c in 0..bytes_per_col {
                let byte = strip[(x * bytes_per_col + c) as usize];
                for b in 0..8u32 {
                    if byte & (0x80 >> b) == 0 {
                        continue;
                    }
                    let row = row_offset + c * 8 + b;
                    let out_index = (row * width_bytes + x / 8) as usize;
                    image.rows[out_index] |= 0x80 >> (x % 8);
                }
            }
        }

        Some(5 + (width * bytes_per_col) as usize + 1)
    }

    fn parse_barcode(&self, i: usize, events: &mut Vec<Event>) -> Option<usize> {
        let data = &self.buf[i..];
        let identifier = *data.get(2)?;
        let symbology = barcode_symbology_name(identifier);

        if identifier > 0x40 {
            let len = *data.get(3)? as usize;
            let bytes = data.get(4..4 + len)?;
            events.push(Event::Barcode { symbology, data: crate::codepage::decode_latin1(bytes) });
            Some(4 + len)
        } else {
            let start = 3;
            let end = start + data[start..].iter().position(|&b| b == 0)?;
            events.push(Event::Barcode { symbology, data: crate::codepage::decode_latin1(&data[start..end]) });
            Some(end + 1)
        }
    }

    /// `GS v 0`: raster bit image. Format: mode, xL, xH (width in *bytes*),
    /// yL, yH (height in dots), then packed 1-bit-per-pixel row data.
    fn parse_raster_image(&self, i: usize, events: &mut Vec<Event>) -> Option<usize> {
        let data = &self.buf[i..];
        data.get(2)?; // '0'
        let _mode = *data.get(3)?;
        let x_l = *data.get(4)? as u32;
        let x_h = *data.get(5)? as u32;
        let y_l = *data.get(6)? as u32;
        let y_h = *data.get(7)? as u32;
        let width_bytes = x_l + x_h * 256;
        let height = y_l + y_h * 256;
        let total = (width_bytes * height) as usize;
        let bits = data.get(8..8 + total)?;

        events.push(Event::Image {
            width: width_bytes * 8,
            height,
            bits: base64::engine::general_purpose::STANDARD.encode(bits),
        });
        Some(8 + total)
    }

    /// `GS ( k pL pH cn fn [params]` — the 2D symbol family (QR/PDF417/etc).
    /// `pL`/`pH` count bytes from `cn` onward, so the whole command is
    /// `5 + pL + 256*pH` bytes.
    fn parse_2d_symbol(&mut self, i: usize, events: &mut Vec<Event>) -> Option<usize> {
        let data = &self.buf[i..];
        let p_l = *data.get(3)? as usize;
        let p_h = *data.get(4)? as usize;
        let total_len = p_l + p_h * 256;
        let consumed = 5 + total_len;
        let params = data.get(5..consumed)?;

        let cn = *params.first()?;
        let fn_ = *params.get(1)?;

        if cn == 0x31 {
            // QR code
            match fn_ {
                0x50 => {
                    // store data: params[2] is a fixed sub-function byte (0x30)
                    let qr_bytes = params.get(3..)?;
                    self.pending_qr_data = Some(crate::codepage::decode_latin1(qr_bytes));
                }
                0x51 => {
                    if let Some(qr_data) = self.pending_qr_data.take() {
                        events.push(Event::QrCode { data: qr_data });
                    }
                }
                _ => {}
            }
        }

        Some(consumed)
    }
}

fn barcode_symbology_name(identifier: u8) -> &'static str {
    match identifier {
        0x00 => "UPC-A",
        0x01 => "UPC-E",
        0x02 => "EAN-13",
        0x03 => "EAN-8",
        0x04 => "CODE39",
        0x05 => "ITF",
        0x06 => "CODABAR",
        0x48 => "CODE93",
        0x49 => "CODE128",
        0x4b => "GS1 DataBar Omni",
        0x4c => "GS1 DataBar Truncated",
        0x4d => "GS1 DataBar Limited",
        0x4e => "GS1 DataBar Expanded",
        0x4f => "CODE128",
        _ => "barcode",
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_newline_cr_from_real_encoder() {
        // Exact bytes @point-of-sale/receipt-printer-encoder produces for
        // .text("line one\nline two").newline() with its default '\n\r'
        // newline setting.
        let bytes = [
            0x1b, 0x40, 0x1c, 0x2e, 0x1b, 0x4d, 0x00, 0x1b, 0x74, 0x00, b'l', b'i', b'n', b'e', b' ', b'o', b'n',
            b'e', 0x0a, 0x0d, b'l', b'i', b'n', b'e', b' ', b't', b'w', b'o', 0x0a, 0x0d,
        ];
        let mut parser = Parser::new();
        let events = parser.feed(&bytes);

        let texts: Vec<&str> =
            events.iter().filter_map(|e| if let Event::Text { text } = e { Some(text.as_str()) } else { None }).collect();
        assert_eq!(texts, vec!["line one", "line two"]);

        let newlines = events.iter().filter(|e| matches!(e, Event::NewLine)).count();
        assert_eq!(newlines, 2, "events: {events:?}");
    }
}
