import index from "./index.html";

const PORT = 4300;

Bun.serve({
  port: PORT,
  routes: {
    "/": index,
    "/assets/receipt-printer.wav": () =>
      new Response(Bun.file("./assets/receipt-printer.wav")),
    "/assets/receipt-cutter.wav": () =>
      new Response(Bun.file("./assets/receipt-cutter.wav")),
  },
  development: {
    hmr: true,
    console: true,
  },
});

console.log(`Virtual ESC/POS printer page: http://localhost:${PORT}`);
console.log(`(Start the gadget daemon separately: cd gadget && sudo ./target/release/escpos_usb_gadget)`);
