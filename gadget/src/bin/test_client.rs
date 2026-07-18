//! Host-side test client: opens the virtual ESC/POS printer over real USB
//! (via nusb, the same kind of transfer a browser's WebUSB implementation
//! performs) and sends a sample receipt exercising styling, an image, a QR
//! code, a barcode, and a cash drawer pulse. Lets us verify the whole
//! pipeline without needing an actual browser click (WebUSB requires a user
//! gesture, so it can't be scripted from a test).

use std::{thread, time::Duration};

use nusb::{
    transfer::{Bulk, Out},
    MaybeFuture,
};

const VENDOR_ID: u16 = 0x0519;
const PRODUCT_ID: u16 = 0x000b;

struct Receipt(Vec<u8>);

impl Receipt {
    fn new() -> Self {
        let mut r = Self(Vec::new());
        r.raw(&[0x1b, 0x40]); // ESC @ init
        r
    }
    fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.0.extend_from_slice(bytes);
        self
    }
    fn align(&mut self, n: u8) -> &mut Self {
        self.raw(&[0x1b, 0x61, n])
    }
    fn bold(&mut self, on: bool) -> &mut Self {
        self.raw(&[0x1b, 0x45, on as u8])
    }
    fn italic(&mut self, on: bool) -> &mut Self {
        self.raw(&[0x1b, 0x34, on as u8])
    }
    fn underline(&mut self, on: bool) -> &mut Self {
        self.raw(&[0x1b, 0x2d, on as u8])
    }
    fn invert(&mut self, on: bool) -> &mut Self {
        self.raw(&[0x1d, 0x42, on as u8])
    }
    /// width/height: 1-8
    fn size(&mut self, width: u8, height: u8) -> &mut Self {
        self.raw(&[0x1d, 0x21, ((width - 1) << 4) | (height - 1)])
    }
    fn cp437(&mut self, bytes: &[u8]) -> &mut Self {
        self.raw(bytes)
    }
    fn text(&mut self, s: &str) -> &mut Self {
        self.raw(s.as_bytes())
    }
    fn line(&mut self, s: &str) -> &mut Self {
        self.text(s).raw(b"\n")
    }
    fn cut(&mut self) -> &mut Self {
        self.raw(&[0x1d, 0x56, 0x00])
    }
    fn pulse(&mut self) -> &mut Self {
        self.raw(&[0x1b, 0x70, 0x00, 50, 250])
    }

    /// GS v 0 raster image. `rows` is a slice of packed 1bpp rows (MSB first),
    /// width must be a multiple of 8.
    fn image(&mut self, width: u32, height: u32, rows: &[u8]) -> &mut Self {
        let width_bytes = width / 8;
        self.raw(&[0x1d, 0x76, 0x30, 0x00]);
        self.raw(&(width_bytes as u16).to_le_bytes());
        self.raw(&(height as u16).to_le_bytes());
        self.raw(rows);
        self
    }

    /// ESC * column-mode image — what `@point-of-sale/receipt-printer-encoder`
    /// actually defaults to for printer models it doesn't recognize (like
    /// our simulated BSC10), rather than GS v 0 raster.
    fn column_image(&mut self, width: u32, height: u32, get_pixel: impl Fn(u32, u32) -> bool) -> &mut Self {
        self.raw(&[0x1b, 0x33, 0x24]); // 24-dot line spacing
        let strips = height.div_ceil(24);
        for s in 0..strips {
            let mut bytes = vec![0u8; (width * 3) as usize];
            for x in 0..width {
                for c in 0..3u32 {
                    for b in 0..8u32 {
                        let y = s * 24 + b + 8 * c;
                        if y < height && get_pixel(x, y) {
                            bytes[(x * 3 + c) as usize] |= 0x80 >> b;
                        }
                    }
                }
            }
            self.raw(&[0x1b, 0x2a, 0x21]);
            self.raw(&(width as u16).to_le_bytes());
            self.raw(&bytes);
            self.raw(b"\n");
        }
        self.raw(&[0x1b, 0x32]); // reset line spacing
        self
    }

    fn qrcode(&mut self, data: &str) -> &mut Self {
        let bytes = data.as_bytes();
        let len = (bytes.len() + 3) as u16;
        self.raw(&[0x1d, 0x28, 0x6b, 0x04, 0x00, 0x31, 0x41, 0x32, 0x00]); // model 2
        self.raw(&[0x1d, 0x28, 0x6b, 0x03, 0x00, 0x31, 0x43, 0x06]); // size 6
        self.raw(&[0x1d, 0x28, 0x6b, 0x03, 0x00, 0x31, 0x45, 0x31]); // error level M
        self.raw(&[0x1d, 0x28, 0x6b]);
        self.raw(&len.to_le_bytes());
        self.raw(&[0x31, 0x50, 0x30]);
        self.raw(bytes);
        self.raw(&[0x1d, 0x28, 0x6b, 0x03, 0x00, 0x31, 0x51, 0x30]); // print
        self
    }

    /// Function B barcode (identifier > 0x40): CODE128.
    fn barcode_code128(&mut self, data: &str) -> &mut Self {
        let bytes = data.as_bytes();
        self.raw(&[0x1d, 0x68, 60]); // height
        self.raw(&[0x1d, 0x77, 2]); // width
        self.raw(&[0x1d, 0x48, 0x02]); // print text below
        self.raw(&[0x1d, 0x6b, 0x49, bytes.len() as u8]);
        self.raw(bytes);
        self
    }
}

/// A small test pattern: a black diamond outline.
fn diamond_pixel(w: u32, h: u32, x: u32, y: u32) -> bool {
    let (cx, cy) = (w as i32 / 2, h as i32 / 2);
    let d = (x as i32 - cx).abs() * h as i32 + (y as i32 - cy).abs() * w as i32;
    d < (w as i32 * h as i32 / 3) && d > (w as i32 * h as i32 / 6)
}

/// Packs `diamond_pixel` as GS v 0 expects (1 bit per pixel, MSB first, row-major).
fn test_image() -> (u32, u32, Vec<u8>) {
    let (w, h) = (64u32, 32u32);
    let mut rows = vec![0u8; ((w / 8) * h) as usize];
    for y in 0..h {
        for x in 0..w {
            if diamond_pixel(w, h, x, y) {
                let byte_index = (y * (w / 8) + (x / 8)) as usize;
                rows[byte_index] |= 0x80 >> (x % 8);
            }
        }
    }
    (w, h, rows)
}

fn main() {
    env_logger::init();

    let device_info = nusb::list_devices()
        .wait()
        .expect("failed to list USB devices")
        .find(|d| d.vendor_id() == VENDOR_ID && d.product_id() == PRODUCT_ID)
        .unwrap_or_else(|| {
            panic!(
                "virtual printer not found (looking for {VENDOR_ID:04x}:{PRODUCT_ID:04x}) — \
                 is the gadget daemon running?"
            )
        });

    println!("found device: {device_info:?}");
    let device = device_info.open().wait().expect("failed to open device");
    let cfg = device.active_configuration().expect("no active configuration");

    let mut interface_number = None;
    let mut ep_out_addr = None;

    for desc in cfg.interface_alt_settings() {
        for ep in desc.endpoints() {
            if matches!(ep.direction(), nusb::transfer::Direction::Out) {
                ep_out_addr = Some(ep.address());
                interface_number = Some(desc.interface_number());
            }
        }
    }

    let interface_number = interface_number.expect("no OUT endpoint found");
    let ep_out_addr = ep_out_addr.expect("no OUT endpoint found");

    let interface = device.claim_interface(interface_number).wait().expect("cannot claim interface");

    // WebUSBReceiptPrinter (and other client libraries) call device.reset()
    // right after claiming the interface — reproduce that here so this test
    // actually exercises the reset-recovery path in the daemon.
    println!("resetting device (mirrors what real client libraries do)...");
    device.reset().wait().expect("reset failed");
    thread::sleep(Duration::from_millis(300));

    let ep_out = interface.endpoint::<Bulk, Out>(ep_out_addr).expect("cannot open OUT endpoint");
    let mut writer = ep_out.writer(4096);

    let mut receipt = Receipt::new();
    receipt
        .align(1)
        .bold(true)
        .size(2, 2)
        .line("Corner Cafe")
        .size(1, 1)
        .bold(false)
        .line("123 Main St, Helsinki")
        .align(0)
        .cp437(&[0x04; 44])
        .raw(b"\n")
        .line("Cappuccino x2                         9.00")
        .italic(true)
        .line("Croissant (imported)                   3.20")
        .italic(false)
        .underline(true)
        .line("Orange Juice                           3.90")
        .underline(false)
        .invert(true)
        .text(" SPECIAL ")
        .invert(false)
        .raw(b"\n")
        .cp437(&[0x04; 44])
        .raw(b"\n")
        .bold(true)
        .line("TOTAL                                 16.10")
        .bold(false);

    let (iw, ih, ibits) = test_image();
    receipt.align(1).column_image(iw, ih, |x, y| diamond_pixel(iw, ih, x, y));
    receipt.line("");
    receipt.image(iw, ih, &ibits);

    receipt
        .qrcode("https://example.com/receipt/12345")
        .barcode_code128("12345678")
        .line("Thank you for visiting!")
        .pulse()
        .raw(b"\n\n\n")
        .cut();

    use std::io::Write;
    writer.write_all(&receipt.0).expect("write failed");
    writer.flush().expect("flush failed");

    println!("sent {} bytes to endpoint 0x{ep_out_addr:02x}", receipt.0.len());
    thread::sleep(Duration::from_millis(200));
}
