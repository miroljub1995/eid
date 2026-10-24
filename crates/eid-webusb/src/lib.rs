//! [`Platform`](eid_core::Platform) over WebUSB: talks CCID straight to a USB smart card reader
//! from the browser. Only does anything on wasm32; the CCID codec builds everywhere for its tests.

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod ccid;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
pub use web::{WebUsb, WebUsbCard, is_supported, request_reader};
