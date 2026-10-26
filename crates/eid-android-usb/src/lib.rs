//! [`Platform`](eid_core::Platform) over Android's USB host API: lists USB smart card readers, asks
//! the user's permission for each (the system dialog), and talks CCID to them through `eid-ccid`.
//!
//! Calls block, so drive it from its own thread (e.g. with `pollster`). Needs an app built on
//! `android-activity` (eframe's Android support), which publishes the VM and Activity it uses.

#[cfg(target_os = "android")]
mod usb;

#[cfg(target_os = "android")]
pub use usb::{Android, AndroidCard};
