<img width="1900" height="1000" alt="Simulator screenshot" src="https://github.com/user-attachments/assets/05a808c7-c826-4efa-b6ae-b561f8556672" />

# USB ESC/POS printer simulator

This is a small Rust program that creates a virtual USB device that looks
like a Star Micronics receipt printer to your computer. This tool can
be useful for testing receipt printing functionality in environments like
WebUSB. The "simulator" itself is a small web app that has a nice animated
virtual receipt printer.

Please note that the styling and/or decoding of text, especially non-ASCII characters,
isn't perfect but is likely enough for 99 % of cases.

This software is licensed under MIT and provided without warranty. The printing and
cutting sounds are split from “Thermal Receipt Print & Cut” by twisterad3, released
under CC0; see `assets/README.md`. You have to click the receipt printer window
at least once to enable audio.

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
- `ESC a n` — align
- `ESC E n` — bold, `ESC - n` — underline, `ESC 4 n` — italic, `GS B n` —
  invert, `GS ! n` — double width/height (1×–8× each), `ESC ! n` — the
  older combined print-mode byte, also supported
- `ESC t n` — codepage select. Text defaults to **CP437** (the ESC/POS
  default), and switches to **Windows-1252** for `ESC t 16` — the index the
  encoder's `autoEncode` picks for the Star BSC10 profile when text needs a
  character CP437 can't represent, most commonly the Euro sign (`€`). Other
  indices fall back to CP437 rather than desyncing the stream; if you hit
  another codepage in practice, `gadget/src/codepage.rs` is where to add it.
- `GS v 0` — raster image, rendered on a canvas
- `GS ( k` — QR codes (rendered with the `qrcode` package) and PDF417
  (parsed, not rendered)
- `GS k` — 1D barcodes (rendered with `JsBarcode`; symbologies it doesn't
  support, like CODE93 or the GS1 DataBar variants, fall back to a text
  label)
- `ESC p` — cash drawer pulse (flashes a little drawer under the printer)
- `GS V m` — cut
- `ESC * ... LF` — column-mode images: consumed correctly so the byte
  stream stays in sync, but not rendered (`GS v 0` raster mode is)

Text bytes that aren't part of a recognized command are decoded through the
CP437 table in `src/codepage.rs` rather than as UTF-8 — that's what makes
things like `♦`/`•`/box-drawing dividers show up correctly instead of as
garbage characters.

## Disclosure

The software is provided "as is", without warranty of any kind.
