mod codepage;
mod protocol;
mod ws;

use std::{thread, time::Duration};

use bytes::BytesMut;
use usb_gadget::{
    default_udc,
    function::custom::{Custom, Endpoint, EndpointDirection, Interface},
    Class, Config, Gadget, Id, OsDescriptor, Strings, WebUsb,
};

// Star Micronics BSC10
const VENDOR_ID: u16 = 0x0519;
const PRODUCT_ID: u16 = 0x000b;

const WS_ADDR: &str = "127.0.0.1:9101";
const FRONTEND_URL: &str = "http://localhost:4300";

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    usb_gadget::remove_all().expect("cannot remove existing gadgets (are you root?)");

    let (mut ep_out_rx, ep_out_dir) = EndpointDirection::host_to_device();
    let (_ep_in_tx, ep_in_dir) = EndpointDirection::device_to_host();

    let (mut custom, handle) = Custom::builder()
        .with_interface(
            Interface::new(Class::vendor_specific(0x00, 0x00), "ESC/POS Printer")
                .with_endpoint(Endpoint::bulk(ep_out_dir))
                .with_endpoint(Endpoint::bulk(ep_in_dir)),
        )
        .build();

    let udc = default_udc().expect(
        "no USB device controller found — load the dummy_hcd kernel module first: \
         sudo modprobe libcomposite dummy_hcd usb_f_fs",
    );

    let reg = Gadget::new(
        Class::INTERFACE_SPECIFIC,
        Id::new(VENDOR_ID, PRODUCT_ID),
        Strings::new("Star Micronics", "BSC10", "SIMULATED-0001"),
    )
    .with_config(Config::new("ESC/POS Printer").with_function(handle))
    .with_os_descriptor(OsDescriptor::microsoft())
    .with_web_usb(WebUsb::new(0x01, FRONTEND_URL))
    .bind(&udc)
    .expect("cannot bind gadget to UDC");

    println!("Virtual ESC/POS printer is live on {}", udc.name().to_string_lossy());
    println!();
    println!("  Vendor ID:  0x{VENDOR_ID:04x}");
    println!("  Product ID: 0x{PRODUCT_ID:04x}");
    println!();
    println!("Your web app can find it with:");
    println!(
        "  navigator.usb.requestDevice({{ filters: [{{ vendorId: 0x{VENDOR_ID:04x}, productId: 0x{PRODUCT_ID:04x} }}] }})"
    );
    println!();
    println!("Animation frontend should run at {FRONTEND_URL}");
    println!();

    let out_control = ep_out_rx.control().expect("no control for OUT endpoint");
    println!("  Bulk OUT endpoint (write ESC/POS bytes here): 0x{:02x}", out_control.real_address().unwrap());
    println!();

    let broadcaster = ws::Broadcaster::new();

    let reader_broadcaster = broadcaster.clone();
    thread::spawn(move || {
        let mut parser = protocol::Parser::new();
        let mut receipt_events = Vec::new();
        let packet_size = ep_out_rx.max_packet_size().unwrap_or(64);

        loop {
            match ep_out_rx.recv_timeout(BytesMut::with_capacity(packet_size), Duration::from_secs(3600)) {
                Ok(Some(data)) => {
                    log::debug!("received {} bytes", data.len());
                    for event in parser.feed(&data) {
                        match event {
                            // Drawer kicks are independent of receipt output and
                            // must reach the frontend even when no cut ever follows.
                            protocol::Event::Pulse => {
                                reader_broadcaster.publish(vec![protocol::Event::Pulse]);
                            }
                            protocol::Event::Cut => {
                                receipt_events.push(protocol::Event::Cut);
                                reader_broadcaster.publish(std::mem::take(&mut receipt_events));
                            }
                            event => receipt_events.push(event),
                        }
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    // The host issuing a USB reset (many WebUSB client libraries call
                    // device.reset() right after claimInterface) disables the endpoint
                    // briefly while the gadget re-enumerates. That surfaces here as a
                    // transfer error — it isn't fatal, so keep the reader alive instead
                    // of exiting and silently dropping every future print job.
                    log::debug!("OUT endpoint read error (likely a bus reset, retrying): {err}");
                    thread::sleep(Duration::from_millis(50));
                }
            }
        }
    });

    thread::spawn(move || loop {
        match custom.event_timeout(Duration::from_secs(1)) {
            Ok(Some(event)) => log::debug!("gadget event: {event:?}"),
            Ok(None) => {}
            Err(err) => {
                log::warn!("gadget event loop error: {err}");
                break;
            }
        }
    });

    ws::serve(WS_ADDR, broadcaster).expect("WebSocket server failed");

    reg.remove().ok();
}
