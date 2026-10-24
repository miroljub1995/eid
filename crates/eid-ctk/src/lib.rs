//! [`Platform`](eid_core::Platform) over Apple's CryptoTokenKit, the way iOS and iPadOS apps reach
//! smart card readers (USB-C or Lightning, iPadOS/iOS 16+). The same API exists on macOS, where the
//! app needs the `com.apple.security.smartcard` entitlement.
//!
//! Calls block until CryptoTokenKit answers, so drive it from its own thread (e.g. with `pollster`).

#[cfg(any(target_os = "ios", target_os = "macos"))]
mod apple;

#[cfg(any(target_os = "ios", target_os = "macos"))]
pub use apple::{CryptoTokenKit, CryptoTokenKitCard};
