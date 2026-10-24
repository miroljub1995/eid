//! WebUSB reached through `js_sys::Reflect`, so the build needs no `web_sys_unstable_apis` flag.

use std::cell::Cell;
use std::rc::Rc;

use eid_core::{Error, Platform, Reader, SmartCard};
use js_sys::{Array, DataView, Function, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use crate::ccid::{self, GET_SLOT_STATUS, Header};

const SMART_CARD_CLASS: u8 = 0x0B;
/// Room for an extended APDU response and its header in one transfer.
const MAX_MESSAGE: u32 = 65544 + ccid::HEADER_LEN as u32;

/// Whether this browser has WebUSB (Chromium-based browsers in a secure context).
pub fn is_supported() -> bool {
    usb().is_ok()
}

/// Opens the browser's device chooser for smart card readers; call it from a click handler, the
/// browser needs user activation. `Ok(false)` means the user closed the chooser without picking.
/// A picked reader shows up in the next [`Platform::readers`] call.
pub async fn request_reader() -> Result<bool, Error> {
    let filter = Object::new();
    set(&filter, "classCode", SMART_CARD_CLASS.into());
    let options = Object::new();
    set(&options, "filters", Array::of1(&filter).into());
    match call_raw(&usb()?, "requestDevice", &[options.into()]).await {
        Ok(_) => Ok(true),
        Err(error) if text(&error, "name").as_deref() == Some("NotFoundError") => Ok(false),
        Err(error) => Err(js_error("requestDevice", &error)),
    }
}

/// The readers the user has granted this site, kept open between listings.
#[derive(Default)]
pub struct WebUsb {
    devices: Vec<Device>,
}

struct Device {
    usb: JsValue,
    name: String,
    link: Link,
}

impl WebUsb {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Platform for WebUsb {
    type Card = WebUsbCard;

    async fn readers(&mut self, wait_ms: u32) -> Result<Vec<Reader>, Error> {
        // shortcut: polls instead of listening to connect/disconnect events and the interrupt
        // endpoint, fine at the service's 500 ms pace.
        sleep(wait_ms).await;
        let permitted: Array = call(&usb()?, "getDevices", &[])
            .await?
            .dyn_into()
            .map_err(|_| Error::Transport("getDevices returned no list".into()))?;
        self.devices
            .retain(|device| permitted.iter().any(|p| Object::is(&p, &device.usb)));

        let mut readers: Vec<Reader> = Vec::new();
        let mut failure = None;
        for usb_device in permitted.iter() {
            let index = match self.devices.iter().position(|d| Object::is(&d.usb, &usb_device)) {
                Some(index) => index,
                None => match open(&usb_device).await {
                    Ok(Some(link)) => {
                        self.devices.push(Device {
                            usb: usb_device.clone(),
                            name: String::new(),
                            link,
                        });
                        self.devices.len() - 1
                    }
                    Ok(None) => continue,
                    Err(error) => {
                        failure = Some(error);
                        continue;
                    }
                },
            };
            let Some(device) = self.devices.get_mut(index) else {
                continue;
            };
            match device.link.exchange(GET_SLOT_STATUS, &[]).await {
                Ok((header, _)) => {
                    let base = text(&usb_device, "productName")
                        .filter(|name| !name.trim().is_empty())
                        .unwrap_or_else(|| "USB smart card reader".into());
                    let mut name = base.clone();
                    let mut n = 1;
                    while readers.iter().any(|r| r.name == name) {
                        n += 1;
                        name = format!("{base} ({n})");
                    }
                    device.name = name.clone();
                    readers.push(Reader {
                        name,
                        has_card: !header.card_absent(),
                    });
                }
                Err(error) => {
                    // Opened afresh next time.
                    self.devices.remove(index);
                    failure = Some(error);
                }
            }
        }
        match failure {
            Some(error) if readers.is_empty() => Err(error),
            _ => Ok(readers),
        }
    }

    async fn connect(&mut self, reader: &str) -> Result<WebUsbCard, Error> {
        let link = self
            .devices
            .iter()
            .find(|d| d.name == reader)
            .map(|d| d.link.clone())
            .ok_or_else(|| Error::Transport(format!("the reader {reader} is gone")))?;
        if !link.apdu_level {
            // shortcut: TPDU-level readers need T=0/T=1 framing done here; add it if one turns up.
            return Err(Error::Transport(
                "this reader only supports TPDU exchange, which is not implemented in the browser".into(),
            ));
        }
        // bPowerSelect 0: the reader picks the voltage.
        let (_, atr) = link.exchange(ccid::ICC_POWER_ON, &[]).await?;
        Ok(WebUsbCard { link, atr })
    }

    async fn sleep(&mut self, ms: u32) {
        sleep(ms).await;
    }
}

pub struct WebUsbCard {
    link: Link,
    atr: Vec<u8>,
}

impl SmartCard for WebUsbCard {
    fn atr(&self) -> &[u8] {
        &self.atr
    }

    async fn transmit(&mut self, apdu: &[u8]) -> Result<Vec<u8>, Error> {
        Ok(self.link.exchange(ccid::XFR_BLOCK, apdu).await?.1)
    }
}

/// An opened, claimed CCID interface.
#[derive(Clone)]
struct Link {
    device: JsValue,
    ep_in: u8,
    ep_out: u8,
    /// Shared by the reader and its card handles so bSeq never repeats between them.
    seq: Rc<Cell<u8>>,
    apdu_level: bool,
}

impl Link {
    /// Sends a PC_to_RDR message (all-zero type-specific bytes) and returns the matching reply.
    async fn exchange(&self, kind: u8, data: &[u8]) -> Result<(Header, Vec<u8>), Error> {
        let seq = self.seq.get();
        self.seq.set(seq.wrapping_add(1));
        let message = Uint8Array::from(ccid::command(kind, seq, [0; 3], data).as_slice());
        let sent = call(&self.device, "transferOut", &[self.ep_out.into(), message.into()]).await?;
        transfer_ok(&sent)?;

        // shortcut: WebUSB transfers have no timeout, so a reader that never answers hangs the read.
        let mut buffer = Vec::new();
        loop {
            let received = call(
                &self.device,
                "transferIn",
                &[self.ep_in.into(), MAX_MESSAGE.into()],
            )
            .await?;
            transfer_ok(&received)?;
            buffer.extend(bytes(&received)?);
            let Some(header) = Header::parse(&buffer) else {
                continue;
            };
            if header.total() > MAX_MESSAGE as usize {
                return Err(Error::Transport("the reader sent a malformed reply".into()));
            }
            if buffer.len() < header.total() {
                continue;
            }
            let body = buffer
                .get(ccid::HEADER_LEN..header.total())
                .unwrap_or_default()
                .to_vec();
            buffer.clear();
            if header.seq != seq || header.time_extension() {
                continue;
            }
            let expected = if kind == GET_SLOT_STATUS {
                ccid::SLOT_STATUS
            } else {
                ccid::DATA_BLOCK
            };
            if header.kind != expected {
                return Err(Error::Transport(format!(
                    "the reader sent an unexpected {:02X} reply",
                    header.kind
                )));
            }
            // A slot status reply is useful even when it reports a failure (e.g. no card).
            if header.failed() && kind != GET_SLOT_STATUS {
                return Err(Error::Transport(reader_error(&header)));
            }
            return Ok((header, body));
        }
    }
}

fn reader_error(header: &Header) -> String {
    if header.card_absent() {
        return "the card was removed".into();
    }
    match header.error {
        0xFE => "the card does not answer".into(),
        0xFB => "the reader reported a hardware error".into(),
        0xFD | 0xFC => "the reader could not talk to the card (parity error)".into(),
        error => format!("the reader reported error {error:02X}"),
    }
}

/// Opens the device and claims its smart card interface; `None` if it has none.
async fn open(device: &JsValue) -> Result<Option<Link>, Error> {
    if !get(device, "opened")?.is_truthy() {
        call(device, "open", &[]).await?;
    }
    if get(device, "configuration")?.is_null() {
        call(device, "selectConfiguration", &[1.into()]).await?;
    }
    let interfaces: Array = get(&get(device, "configuration")?, "interfaces")?
        .dyn_into()
        .map_err(|_| Error::Transport("the reader has no interfaces".into()))?;
    for interface in interfaces.iter() {
        let alternate = get(&interface, "alternate")?;
        if get(&alternate, "interfaceClass")?.as_f64() != Some(SMART_CARD_CLASS.into()) {
            continue;
        }
        let number = get(&interface, "interfaceNumber")?.as_f64().unwrap_or_default() as u8;
        if !get(&interface, "claimed")?.is_truthy() {
            call_raw(device, "claimInterface", &[number.into()])
                .await
                .map_err(|error| {
                    // Chrome counts smart card readers as a protected USB class: ordinary pages may not
                    // claim them, only Isolated Web Apps with the usb-unrestricted permission.
                    if text(&error, "name").as_deref() == Some("SecurityError") {
                        Error::NotAllowed(message(&error))
                    } else {
                        Error::Transport(format!(
                            "could not claim the reader ({}); it is probably in use by the system smart card service",
                            message(&error)
                        ))
                    }
                })?;
        }
        let endpoints: Array = get(&alternate, "endpoints")?
            .dyn_into()
            .map_err(|_| Error::Transport("the reader has no endpoints".into()))?;
        let bulk = |direction: &str| {
            endpoints
                .iter()
                .find(|e| {
                    text(e, "type").as_deref() == Some("bulk")
                        && text(e, "direction").as_deref() == Some(direction)
                })
                .and_then(|e| get(&e, "endpointNumber").ok())
                .and_then(|n| n.as_f64())
                .map(|n| n as u8)
                .ok_or_else(|| Error::Transport(format!("the reader has no bulk {direction} endpoint")))
        };
        let (ep_in, ep_out) = (bulk("in")?, bulk("out")?);
        return Ok(Some(Link {
            device: device.clone(),
            ep_in,
            ep_out,
            seq: Rc::default(),
            apdu_level: apdu_level(device, number).await,
        }));
    }
    Ok(None)
}

/// Reads dwFeatures from the CCID class descriptor; assumes APDU level if that fails.
async fn apdu_level(device: &JsValue, interface: u8) -> bool {
    let setup = Object::new();
    set(&setup, "requestType", "standard".into());
    set(&setup, "recipient", "device".into());
    set(&setup, "request", 6.into()); // GET_DESCRIPTOR
    set(&setup, "value", 0x0200.into()); // CONFIGURATION, index 0
    set(&setup, "index", 0.into());
    match call(device, "controlTransferIn", &[setup.into(), 512.into()]).await {
        Ok(result) => bytes(&result)
            .ok()
            .and_then(|config| ccid::apdu_level(&config, interface))
            .unwrap_or(true),
        Err(_) => true,
    }
}

async fn sleep(ms: u32) {
    let promise = Promise::new(&mut |resolve, _| {
        let global = js_sys::global();
        let started = get(&global, "setTimeout")
            .ok()
            .and_then(|f| f.dyn_into::<Function>().ok())
            .is_some_and(|f| f.call2(&global, &resolve, &ms.into()).is_ok());
        if !started {
            let _ = resolve.call0(&JsValue::UNDEFINED);
        }
    });
    let _ = JsFuture::from(promise).await;
}

fn usb() -> Result<JsValue, Error> {
    let usb = get(&js_sys::global(), "navigator").and_then(|navigator| get(&navigator, "usb"));
    match usb {
        Ok(usb) if usb.is_object() => Ok(usb),
        _ => Err(Error::Transport(
            "this browser has no WebUSB; use Chrome or Edge over HTTPS".into(),
        )),
    }
}

fn transfer_ok(result: &JsValue) -> Result<(), Error> {
    match text(result, "status").as_deref() {
        Some("ok") => Ok(()),
        status => Err(Error::Transport(format!(
            "the USB transfer ended with {}",
            status.unwrap_or("no status")
        ))),
    }
}

/// The bytes of a USBInTransferResult.
fn bytes(result: &JsValue) -> Result<Vec<u8>, Error> {
    let data: DataView = get(result, "data")?
        .dyn_into()
        .map_err(|_| Error::Transport("the USB transfer returned no data".into()))?;
    Ok(Uint8Array::new_with_byte_offset_and_length(
        &data.buffer(),
        data.byte_offset() as u32,
        data.byte_length() as u32,
    )
    .to_vec())
}

fn get(target: &JsValue, key: &str) -> Result<JsValue, Error> {
    Reflect::get(target, &key.into()).map_err(|error| js_error(key, &error))
}

fn set(target: &Object, key: &str, value: JsValue) {
    // Setting a property on a fresh plain object cannot fail.
    let _ = Reflect::set(target, &key.into(), &value);
}

fn text(target: &JsValue, key: &str) -> Option<String> {
    Reflect::get(target, &key.into()).ok()?.as_string()
}

/// Calls a promise-returning method and awaits it, keeping the JS error for the caller.
async fn call_raw(target: &JsValue, method: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let function: Function = Reflect::get(target, &method.into())?
        .dyn_into()
        .map_err(|_| JsValue::from(format!("{method} is not a function")))?;
    let result = function.apply(target, &args.iter().collect::<Array>())?;
    JsFuture::from(Promise::resolve(&result)).await
}

async fn call(target: &JsValue, method: &str, args: &[JsValue]) -> Result<JsValue, Error> {
    call_raw(target, method, args)
        .await
        .map_err(|error| js_error(method, &error))
}

fn js_error(context: &str, error: &JsValue) -> Error {
    Error::Transport(format!("USB {context} failed: {}", message(error)))
}

/// A DOMException or Error's message, or the thrown value itself.
fn message(error: &JsValue) -> String {
    text(error, "message")
        .filter(|m| !m.is_empty())
        .or_else(|| error.as_string())
        .or_else(|| text(error, "name"))
        .unwrap_or_else(|| format!("{error:?}"))
}
