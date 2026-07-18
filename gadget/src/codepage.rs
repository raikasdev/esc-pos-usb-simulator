//! ESC/POS byte-to-Unicode decoding for the Star codepage mapping used by
//! `@point-of-sale/receipt-printer-encoder`.
//!
//! The tables are generated from Niels Leenheer's CodepageEncoder package;
//! see `gadget/tools/generate-codepage-tables.mjs`.

#[path = "codepage_tables.rs"]
mod tables;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codepage {
    Cp437,
    StarKatakana,
    Cp850,
    Cp860,
    Cp863,
    Cp865,
    Windows1252,
    Cp866,
    Cp852,
    Cp858,
    Thai42,
    Thai11,
    Thai13,
    Thai14,
    Thai16,
    Thai18,
}

/// Map `ESC t n` exactly like ReceiptPrinterEncoder's Star ESC/POS mapping.
/// Unknown values use CP437, the ESC/POS default, so they never desynchronize
/// the byte stream.
pub fn codepage_from_star_index(n: u8) -> Codepage {
    match n {
        0x01 => Codepage::StarKatakana,
        0x02 => Codepage::Cp850,
        0x03 => Codepage::Cp860,
        0x04 => Codepage::Cp863,
        0x05 => Codepage::Cp865,
        0x10 => Codepage::Windows1252,
        0x11 => Codepage::Cp866,
        0x12 => Codepage::Cp852,
        0x13 => Codepage::Cp858,
        0x14 => Codepage::Thai42,
        0x15 => Codepage::Thai11,
        0x16 => Codepage::Thai13,
        0x17 => Codepage::Thai14,
        0x18 => Codepage::Thai16,
        0x1a => Codepage::Thai18,
        _ => Codepage::Cp437,
    }
}

fn table(codepage: Codepage) -> &'static [u32; 256] {
    match codepage {
        Codepage::Cp437 => &tables::CP437,
        Codepage::StarKatakana => &tables::STAR_KATAKANA,
        Codepage::Cp850 => &tables::CP850,
        Codepage::Cp860 => &tables::CP860,
        Codepage::Cp863 => &tables::CP863,
        Codepage::Cp865 => &tables::CP865,
        Codepage::Windows1252 => &tables::WINDOWS_1252,
        Codepage::Cp866 => &tables::CP866,
        Codepage::Cp852 => &tables::CP852,
        Codepage::Cp858 => &tables::CP858,
        Codepage::Thai42 => &tables::THAI42,
        Codepage::Thai11 => &tables::THAI11,
        Codepage::Thai13 => &tables::THAI13,
        Codepage::Thai14 => &tables::THAI14,
        Codepage::Thai16 => &tables::THAI16,
        Codepage::Thai18 => &tables::THAI18,
    }
}

/// Decode a run of printable text.
///
/// Bytes below 0x20 are decoded through the table like any other, which is
/// how `cp437(&[0x04; 44])` in the test client draws a rule of `♦`. That
/// permissiveness is deliberate, but it means any command byte the parser
/// fails to consume shows up as a CP437 glyph on the receipt rather than
/// being silently ignored — see the debug log in `Parser::feed`.
pub fn decode(codepage: Codepage, bytes: &[u8]) -> String {
    let codepoints = table(codepage);
    bytes
        .iter()
        .filter_map(|&byte| char::from_u32(codepoints[byte as usize]))
        .collect()
}

/// QR/PDF417 and barcode payloads are encoded as ISO-8859-1 by the reference
/// encoder, where every byte maps directly to the same Unicode codepoint.
pub fn decode_latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| byte as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_mapping_decodes_representative_reference_bytes() {
        assert_eq!(decode(Codepage::Cp437, &[0x04, 0x82]), "♦é");
        assert_eq!(decode(Codepage::Cp858, &[0xd5]), "€");
        assert_eq!(decode(Codepage::Windows1252, &[0x80, 0x93, 0x94]), "€“”");
        assert_eq!(
            decode(Codepage::Cp866, &[0x8f, 0xe0, 0xa8, 0xa2, 0xa5, 0xe2]),
            "Привет"
        );
        assert_eq!(decode(Codepage::StarKatakana, &[0xa6, 0xb1, 0xdd]), "ｦｱﾝ");
    }

    #[test]
    fn star_indices_match_receipt_printer_encoder_mapping() {
        assert_eq!(codepage_from_star_index(0x02), Codepage::Cp850);
        assert_eq!(codepage_from_star_index(0x12), Codepage::Cp852);
        assert_eq!(codepage_from_star_index(0x13), Codepage::Cp858);
        assert_eq!(codepage_from_star_index(0xff), Codepage::Cp437);
    }
}
