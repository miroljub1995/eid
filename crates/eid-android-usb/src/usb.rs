// Adopting the VM and Activity pointers from `ndk-context` needs `unsafe`; see the two blocks below.
#![allow(unsafe_code)]

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use eid_ccid::{Bulk, Ccid};
use eid_core::{Error, Platform, Reader, SmartCard};
use jni::objects::{GlobalRef, JByteArray, JObject, JObjectArray, JString, JValue};
use jni::{JNIEnv, JavaVM};

const SMART_CARD_CLASS: i32 = 0x0B;
const USB_ENDPOINT_XFER_BULK: i32 = 2;
const USB_DIR_IN: i32 = 0x80;
/// Per bulk transfer: a card answers in well under a second; a reader asking for more time sends
/// time-extension messages within this.
const TRANSFER_TIMEOUT_MS: i32 = 5_000;
const PERMISSION_ACTION: &str = "rs.eid.USB_PERMISSION";

pub struct Android {
    vm: Arc<JavaVM>,
    context: GlobalRef,
    open: Vec<Open>,
    /// Readers we already asked permission for, by device path, so the dialog shows once.
    asked: HashSet<String>,
}

struct Open {
    path: String,
    name: String,
    ccid: Arc<Mutex<Ccid<JniBulk>>>,
    apdu_level: bool,
}

impl Android {
    /// Adopts the Java VM and Activity that `android-activity` published for this app.
    pub fn new() -> Result<Self, Error> {
        let ctx = ndk_context::android_context();
        // SAFETY: `android-activity` publishes the process's JavaVM pointer, valid for its lifetime.
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.map_err(jni_error)?;
        let context = {
            let env = vm.attach_current_thread().map_err(jni_error)?;
            // SAFETY: the Activity's global reference, valid while the app runs; we take our own
            // global reference to it and never delete the borrowed one.
            let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
            env.new_global_ref(activity).map_err(jni_error)?
        };
        Ok(Android {
            vm: Arc::new(vm),
            context,
            open: Vec::new(),
            asked: HashSet::new(),
        })
    }

    fn list(&mut self) -> Result<Vec<Reader>, Error> {
        let vm = Arc::clone(&self.vm);
        let mut env = vm.attach_current_thread().map_err(jni_error)?;
        let manager = usb_manager(&mut env, &self.context)?;
        let devices = object(&mut env, &manager, "getDeviceList", "()Ljava/util/HashMap;", &[])?;
        let values = object(&mut env, &devices, "values", "()Ljava/util/Collection;", &[])?;
        let array = JObjectArray::from(object(
            &mut env,
            &values,
            "toArray",
            "()[Ljava/lang/Object;",
            &[],
        )?);
        let count = env.get_array_length(&array).map_err(|e| checked(&mut env, e))?;

        let mut present = Vec::new();
        for i in 0..count {
            let device = env
                .get_object_array_element(&array, i)
                .map_err(|e| checked(&mut env, e))?;
            let Some(interface) = smart_card_interface(&mut env, &device)? else {
                continue;
            };
            let path = string(&mut env, &device, "getDeviceName")?.unwrap_or_default();
            let name =
                string(&mut env, &device, "getProductName")?.unwrap_or_else(|| "USB čitač kartica".into());
            present.push(path.clone());

            if self.open.iter().any(|o| o.path == path) {
                continue;
            }
            let device_value = JValue::Object(&device);
            if !call(
                &mut env,
                &manager,
                "hasPermission",
                "(Landroid/hardware/usb/UsbDevice;)Z",
                &[device_value],
            )?
            .z()
            .map_err(jni_error)?
            {
                if self.asked.insert(path) {
                    request_permission(&mut env, &self.context, &manager, &device)?;
                }
                continue;
            }
            // shortcut: a reader that fails to open is skipped and retried on the next listing.
            if let Ok(open) = open_reader(&mut env, &self.vm, &manager, &device, &interface, path, name) {
                self.open.push(open);
            }
        }
        self.open.retain(|o| present.contains(&o.path));
        self.asked.retain(|path| present.contains(path));

        let mut readers: Vec<Reader> = Vec::new();
        for open in &self.open {
            let has_card = lock(&open.ccid)?.has_card().unwrap_or(false);
            // Two identical readers get distinct names.
            let mut name = open.name.clone();
            let mut n = 1;
            while readers.iter().any(|r| r.name == name) {
                n += 1;
                name = format!("{} ({n})", open.name);
            }
            readers.push(Reader { name, has_card });
        }
        Ok(readers)
    }
}

impl Platform for Android {
    type Card = AndroidCard;

    async fn readers(&mut self, wait_ms: u32) -> Result<Vec<Reader>, Error> {
        // shortcut: polls; a BroadcastReceiver for attach/detach would need a Java class.
        std::thread::sleep(Duration::from_millis(wait_ms.into()));
        self.list()
    }

    async fn connect(&mut self, reader: &str) -> Result<AndroidCard, Error> {
        let readers = self.list()?;
        let index = readers
            .iter()
            .position(|r| r.name == reader)
            .ok_or_else(|| Error::Transport("the reader is gone".into()))?;
        let open = self
            .open
            .get(index)
            .ok_or_else(|| Error::Transport("the reader is gone".into()))?;
        if !open.apdu_level {
            // shortcut: TPDU-level readers would need T=0/T=1 framing here; most current readers
            // exchange whole APDUs.
            return Err(Error::Transport(
                "this reader only exchanges TPDUs, which is not supported".into(),
            ));
        }
        let atr = lock(&open.ccid)?.power_on()?;
        Ok(AndroidCard {
            ccid: Arc::clone(&open.ccid),
            atr,
        })
    }

    async fn sleep(&mut self, ms: u32) {
        std::thread::sleep(Duration::from_millis(ms.into()));
    }
}

/// A powered card; it is powered off again when dropped.
pub struct AndroidCard {
    ccid: Arc<Mutex<Ccid<JniBulk>>>,
    atr: Vec<u8>,
}

impl SmartCard for AndroidCard {
    fn atr(&self) -> &[u8] {
        &self.atr
    }

    async fn transmit(&mut self, apdu: &[u8]) -> Result<Vec<u8>, Error> {
        lock(&self.ccid)?.transmit(apdu)
    }
}

impl Drop for AndroidCard {
    fn drop(&mut self) {
        if let Ok(mut ccid) = lock(&self.ccid) {
            // Best effort: a reader unplugged mid-read can't be powered off anyway.
            ccid.power_off().ok();
        }
    }
}

/// A reader's bulk endpoints, through `UsbDeviceConnection.bulkTransfer`.
struct JniBulk {
    vm: Arc<JavaVM>,
    connection: GlobalRef,
    ep_in: GlobalRef,
    ep_out: GlobalRef,
}

impl JniBulk {
    fn transfer(&self, endpoint: &GlobalRef, buffer: &JByteArray, len: usize) -> Result<usize, Error> {
        let mut env = self.vm.attach_current_thread().map_err(jni_error)?;
        let len = i32::try_from(len).map_err(|_| Error::Transport("transfer too large".into()))?;
        let args = [
            JValue::Object(endpoint.as_obj()),
            JValue::Object(buffer),
            JValue::Int(len),
            JValue::Int(TRANSFER_TIMEOUT_MS),
        ];
        let sig = "(Landroid/hardware/usb/UsbEndpoint;[BII)I";
        let done = call(&mut env, self.connection.as_obj(), "bulkTransfer", sig, &args)?
            .i()
            .map_err(jni_error)?;
        usize::try_from(done)
            .map_err(|_| Error::Transport("the reader did not answer (USB transfer failed)".into()))
    }
}

impl Bulk for JniBulk {
    fn write(&mut self, data: &[u8]) -> Result<(), Error> {
        let mut env = self.vm.attach_current_thread().map_err(jni_error)?;
        let buffer = env
            .byte_array_from_slice(data)
            .map_err(|e| checked(&mut env, e))?;
        let sent = self.transfer(&self.ep_out, &buffer, data.len())?;
        if sent != data.len() {
            return Err(Error::Transport("the reader took only part of a message".into()));
        }
        Ok(())
    }

    fn read(&mut self, max: usize) -> Result<Vec<u8>, Error> {
        let mut env = self.vm.attach_current_thread().map_err(jni_error)?;
        let len = i32::try_from(max).map_err(|_| Error::Transport("transfer too large".into()))?;
        let buffer = env.new_byte_array(len).map_err(|e| checked(&mut env, e))?;
        let received = self.transfer(&self.ep_in, &buffer, max)?;
        let mut bytes = env
            .convert_byte_array(&buffer)
            .map_err(|e| checked(&mut env, e))?;
        bytes.truncate(received);
        Ok(bytes)
    }
}

impl Drop for JniBulk {
    fn drop(&mut self) {
        if let Ok(mut env) = self.vm.attach_current_thread() {
            call(&mut env, self.connection.as_obj(), "close", "()V", &[]).ok();
        }
    }
}

fn usb_manager<'a>(env: &mut JNIEnv<'a>, context: &GlobalRef) -> Result<JObject<'a>, Error> {
    let service = JObject::from(env.new_string("usb").map_err(|e| checked(env, e))?);
    let manager = object(
        env,
        context.as_obj(),
        "getSystemService",
        "(Ljava/lang/String;)Ljava/lang/Object;",
        &[JValue::Object(&service)],
    )?;
    if manager.is_null() {
        return Err(Error::Transport("this device has no USB host support".into()));
    }
    Ok(manager)
}

/// The device's first smart card interface, if it has one.
fn smart_card_interface<'a>(env: &mut JNIEnv<'a>, device: &JObject) -> Result<Option<JObject<'a>>, Error> {
    let count = call(env, device, "getInterfaceCount", "()I", &[])?
        .i()
        .map_err(jni_error)?;
    for i in 0..count {
        let interface = object(
            env,
            device,
            "getInterface",
            "(I)Landroid/hardware/usb/UsbInterface;",
            &[JValue::Int(i)],
        )?;
        if call(env, &interface, "getInterfaceClass", "()I", &[])?
            .i()
            .map_err(jni_error)?
            == SMART_CARD_CLASS
        {
            return Ok(Some(interface));
        }
    }
    Ok(None)
}

fn request_permission(
    env: &mut JNIEnv,
    context: &GlobalRef,
    manager: &JObject,
    device: &JObject,
) -> Result<(), Error> {
    let action = JObject::from(env.new_string(PERMISSION_ACTION).map_err(|e| checked(env, e))?);
    let intent = env
        .new_object(
            "android/content/Intent",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&action)],
        )
        .map_err(|e| checked(env, e))?;
    // Explicit to this app, as Android 14 requires for an immutable PendingIntent.
    let package = object(
        env,
        context.as_obj(),
        "getPackageName",
        "()Ljava/lang/String;",
        &[],
    )?;
    call(
        env,
        &intent,
        "setPackage",
        "(Ljava/lang/String;)Landroid/content/Intent;",
        &[JValue::Object(&package)],
    )?;
    let immutable = env
        .get_static_field("android/app/PendingIntent", "FLAG_IMMUTABLE", "I")
        .and_then(|v| v.i())
        .map_err(|e| checked(env, e))?;
    let pending = env
        .call_static_method(
            "android/app/PendingIntent",
            "getBroadcast",
            "(Landroid/content/Context;ILandroid/content/Intent;I)Landroid/app/PendingIntent;",
            &[
                JValue::Object(context.as_obj()),
                JValue::Int(0),
                JValue::Object(&intent),
                JValue::Int(immutable),
            ],
        )
        .and_then(|v| v.l())
        .map_err(|e| checked(env, e))?;
    // The answer is picked up by polling `hasPermission`, so nothing receives the broadcast.
    let sig = "(Landroid/hardware/usb/UsbDevice;Landroid/app/PendingIntent;)V";
    call(
        env,
        manager,
        "requestPermission",
        sig,
        &[JValue::Object(device), JValue::Object(&pending)],
    )?;
    Ok(())
}

fn open_reader(
    env: &mut JNIEnv,
    vm: &Arc<JavaVM>,
    manager: &JObject,
    device: &JObject,
    interface: &JObject,
    path: String,
    name: String,
) -> Result<Open, Error> {
    let connection = object(
        env,
        manager,
        "openDevice",
        "(Landroid/hardware/usb/UsbDevice;)Landroid/hardware/usb/UsbDeviceConnection;",
        &[JValue::Object(device)],
    )?;
    if connection.is_null() {
        return Err(Error::Transport("could not open the reader".into()));
    }
    let claimed = call(
        env,
        &connection,
        "claimInterface",
        "(Landroid/hardware/usb/UsbInterface;Z)Z",
        &[JValue::Object(interface), JValue::Bool(1)],
    )?
    .z()
    .map_err(jni_error)?;
    if !claimed {
        call(env, &connection, "close", "()V", &[]).ok();
        return Err(Error::Transport("could not claim the reader".into()));
    }

    let (mut ep_in, mut ep_out) = (None, None);
    let count = call(env, interface, "getEndpointCount", "()I", &[])?
        .i()
        .map_err(jni_error)?;
    for i in 0..count {
        let endpoint = object(
            env,
            interface,
            "getEndpoint",
            "(I)Landroid/hardware/usb/UsbEndpoint;",
            &[JValue::Int(i)],
        )?;
        if call(env, &endpoint, "getType", "()I", &[])?
            .i()
            .map_err(jni_error)?
            != USB_ENDPOINT_XFER_BULK
        {
            continue;
        }
        let incoming = call(env, &endpoint, "getDirection", "()I", &[])?
            .i()
            .map_err(jni_error)?
            == USB_DIR_IN;
        let slot = if incoming { &mut ep_in } else { &mut ep_out };
        if slot.is_none() {
            *slot = Some(env.new_global_ref(&endpoint).map_err(jni_error)?);
        }
    }
    let connection = env.new_global_ref(&connection).map_err(jni_error)?;
    let (Some(ep_in), Some(ep_out)) = (ep_in, ep_out) else {
        call(env, connection.as_obj(), "close", "()V", &[]).ok();
        return Err(Error::Transport("the reader has no bulk endpoints".into()));
    };

    let number = call(env, interface, "getId", "()I", &[])?
        .i()
        .map_err(jni_error)?;
    let descriptors = JByteArray::from(object(
        env,
        connection.as_obj(),
        "getRawDescriptors",
        "()[B",
        &[],
    )?);
    // Unknown when the descriptors can't be read: assume the common APDU level.
    let apdu_level = if descriptors.is_null() {
        true
    } else {
        let bytes = env
            .convert_byte_array(&descriptors)
            .map_err(|e| checked(env, e))?;
        let config = configuration(&bytes);
        u8::try_from(number)
            .ok()
            .and_then(|n| eid_ccid::apdu_level(config, n))
            .unwrap_or(true)
    };

    let bulk = JniBulk {
        vm: Arc::clone(vm),
        connection,
        ep_in,
        ep_out,
    };
    Ok(Open {
        path,
        name,
        ccid: Arc::new(Mutex::new(Ccid::new(bulk))),
        apdu_level,
    })
}

/// `getRawDescriptors` starts with the device descriptor; the configuration descriptor follows it.
fn configuration(raw: &[u8]) -> &[u8] {
    let skip = raw.first().map_or(0, |len| usize::from(*len));
    raw.get(skip..).unwrap_or_default()
}

fn call<'a>(
    env: &mut JNIEnv<'a>,
    object: &JObject,
    method: &str,
    sig: &str,
    args: &[JValue],
) -> Result<jni::objects::JValueOwned<'a>, Error> {
    env.call_method(object, method, sig, args)
        .map_err(|e| checked(env, e))
}

fn object<'a>(
    env: &mut JNIEnv<'a>,
    object: &JObject,
    method: &str,
    sig: &str,
    args: &[JValue],
) -> Result<JObject<'a>, Error> {
    call(env, object, method, sig, args)?.l().map_err(jni_error)
}

/// A String-returning getter; `None` for a null result.
fn string(env: &mut JNIEnv, object: &JObject, method: &str) -> Result<Option<String>, Error> {
    let value = call(env, object, method, "()Ljava/lang/String;", &[])?
        .l()
        .map_err(jni_error)?;
    if value.is_null() {
        return Ok(None);
    }
    let value = JString::from(value);
    let text: String = env.get_string(&value).map_err(|e| checked(env, e))?.into();
    Ok(Some(text))
}

/// Clears a pending Java exception, so the next JNI call works, and reports the error.
fn checked(env: &mut JNIEnv, error: jni::errors::Error) -> Error {
    if env.exception_check().unwrap_or(false) {
        env.exception_describe().ok();
        env.exception_clear().ok();
    }
    jni_error(error)
}

fn jni_error(error: jni::errors::Error) -> Error {
    Error::Transport(format!("Android USB: {error}"))
}

fn lock(ccid: &Mutex<Ccid<JniBulk>>) -> Result<std::sync::MutexGuard<'_, Ccid<JniBulk>>, Error> {
    Ok(ccid.lock().unwrap_or_else(PoisonError::into_inner))
}
