# USB ESC/POS printer simulator

This is a small Rust program that creates a virtual USB device that looks
like a Star Micronics receipt printer to your computer. This tool can
be useful for testing receipt printing functionality in environments like
WebUSB. The "simulator" itself is a small web app that has a nice animated
virtual receipt printer.

Please note that the styling and/or decoding of text, especially non-ASCII characters,
isn't perfect but is likely enough for 99 % of cases.

This software is licensed under MIT and provided without warranty. The printing,
cutting, and cash-register sounds are released under CC0; see `assets/README.md`.
You have to click the receipt printer window at least once to enable audio.

For now this application is only supported on Linux, well because it uses Linux kernel modules.

## One-time setup

Load the kernel modules that provide a software-only USB device controller:
```bash
sudo modprobe libcomposite dummy_hcd usb_f_fs
```

Grant regular users access to the simulated device (otherwise only root can
open it, which blocks WebUSB in the browser):

```bash
echo 'SUBSYSTEM=="usb", ATTRS{idVendor}=="0519", ATTRS{idProduct}=="000b", MODE="0666"' \
  | sudo tee /etc/udev/rules.d/99-escpos-simulator.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Both of these need re-doing after a reboot (modules aren't persisted here on
purpose — add them to `/etc/modules-load.d/` yourself if you want them to
survive reboots).

## Running

Two processes, in two terminals:

```bash
# Run the virtual printer device
cd gadget && cargo build --release
sudo ./target/release/escpos_usb_gadget

# 2. Run the frontend (another terminal tab)
bun install
bun run server.ts
```

Then open **http://localhost:4300**. It connects to the daemon over a
WebSocket and shows "printer ready" once both are up.

The **Width** box in the toolbar sets the roll width in millimetres (32–152,
default 80), converted internally to the character columns the layout needs.
The two standard sizes are pinned to their real column counts — 80 mm to 48
columns, 58 mm to 32 — because their unprintable margins differ (8 mm vs
10 mm) and no single formula hits both; other widths are derived at 203 dpi
with a 12-dot font A cell and a nominal 8 mm margin, so treat them as
approximate. The setting is remembered between sessions and applies to the
next receipt; receipts already on the desk keep the width they were printed
at.

## Using it from your POS app

The simulator pretends to be a Star Micronics BSC10 (chosen because it speaks plain
ESC/POS — some other Star printers use Star's own star-line/star-prnt
dialects instead):

- Vendor ID: `0x0519`
- Product ID: `0x000b`

If you're on the web, please check out [@point-of-sale/](https://point-of-sale.dev/) libraries.

## Testing without a browser

You can test the simulator also by just running the Rust test client.

```bash
cd gadget && cargo build --release
./target/release/test_client
```

## ESC/POS commands understood

This simulator has been developed against `@point-of-sale/receipt-printer-encoder` supported ESC/POS syntax.

- `ESC @` / `FS .` — init
- `ESC a n` — align, `ESC M n` — font A/B
- `ESC E n` — bold, `ESC - n` — underline, `ESC 4 n` — italic, `GS B n` —
  invert, `GS ! n` — double width/height (1×–8× each), `ESC ! n` — the
  older combined print-mode byte, also supported
- `ESC t n` — codepage select. Text defaults to **CP437** and supports the
  complete Star ESC/POS mapping used by ReceiptPrinterEncoder: Star Katakana,
  CP850/852/858/860/863/865/866, Windows-1252, and the Star Thai variants.
  The byte-to-Unicode tables are generated directly from the bundled
  CodepageEncoder reference with `gadget/tools/generate-codepage-tables.mjs`.
- `GS v 0` — raster image, rendered on a canvas
- `GS ( k` — QR codes (rendered with the `qrcode` package) and PDF417
  (parsed, not rendered)
- `GS k` — 1D barcodes (rendered with `JsBarcode`; symbologies it doesn't
  support, like CODE93 or the GS1 DataBar variants, fall back to a text
  label)
- `ESC p` — cash drawer pulse (opens the animated drawer and plays a ding;
  produces no receipt output)
- `DLE EOT n`, `DLE ENQ`, `DLE DC4 ...` — real-time status queries and the
  real-time drawer kick. Nothing replies on the IN endpoint, but these are
  consumed rather than printed: client libraries poll them *between* jobs, so
  when they were left unparsed their bytes decoded as CP437 glyphs (`►♣`,
  `►♦☺`) at the top of the next receipt
- `GS V m`, `ESC i`, `ESC m` — cut
- `ESC * ... LF` — column-mode images: consumed correctly so the byte
  stream stays in sync, but not rendered (`GS v 0` raster mode is)

Text bytes that aren't part of a recognized command are decoded through the
selected table in `gadget/src/codepage.rs` rather than as UTF-8 — that's what makes
things like `♦`/`•`/box-drawing dividers show up correctly instead of as
garbage characters.

The flip side is that bytes below 0x20 print as their CP437 glyphs too, so a
command the parser doesn't recognize shows up as stray symbols on the receipt
instead of being ignored. If you ever see those, run the daemon with
`RUST_LOG=debug` — every control byte that reaches the text stream is logged
with its hex value, which identifies the unhandled command.

## Disclosure

The software is provided "as is", without warranty of any kind.
