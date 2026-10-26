// Objective-C interop needs `unsafe`; every block below says why it holds.
#![allow(unsafe_code)]

use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use eid_core::{Error, Platform, Reader, SmartCard};
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_crypto_token_kit::{TKSmartCard, TKSmartCardSlot, TKSmartCardSlotManager, TKSmartCardSlotState};
use objc2_foundation::{NSData, NSError, NSString};

/// How long to wait for CryptoTokenKit to answer one request before giving up on the reader.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

pub struct CryptoTokenKit {
    manager: Option<Retained<TKSmartCardSlotManager>>,
}

impl Default for CryptoTokenKit {
    fn default() -> Self {
        Self::new()
    }
}

impl CryptoTokenKit {
    pub fn new() -> Self {
        // SAFETY: a class method with no arguments; a nil result (macOS without the entitlement)
        // becomes `None`.
        let manager = unsafe { TKSmartCardSlotManager::defaultManager() };
        CryptoTokenKit { manager }
    }

    fn manager(&self) -> Result<&TKSmartCardSlotManager, Error> {
        self.manager.as_deref().ok_or_else(|| {
            Error::Transport(
                "CryptoTokenKit is not available (on macOS the app needs the com.apple.security.smartcard \
                 entitlement)"
                    .into(),
            )
        })
    }

    fn slot(&self, name: &str) -> Result<Option<Retained<TKSmartCardSlot>>, Error> {
        let manager = self.manager()?;
        // SAFETY: `manager` is a live slot manager and the name a valid NSString.
        Ok(unsafe { manager.slotNamed(&NSString::from_str(name)) })
    }
}

impl Platform for CryptoTokenKit {
    type Card = CryptoTokenKitCard;

    async fn readers(&mut self, wait_ms: u32) -> Result<Vec<Reader>, Error> {
        // shortcut: polls; CryptoTokenKit reports slot and card changes through KVO, which would make
        // insertion instant instead of within `wait_ms`.
        std::thread::sleep(Duration::from_millis(wait_ms.into()));
        let manager = self.manager()?;
        // SAFETY: `manager` is a live slot manager.
        let names = unsafe { manager.slotNames() };
        let mut readers = Vec::new();
        for name in names.iter() {
            let name = name.to_string();
            let Some(slot) = self.slot(&name)? else { continue };
            // SAFETY: `slot` is a live slot.
            let state = unsafe { slot.state() };
            if state == TKSmartCardSlotState::Missing {
                continue;
            }
            // A mute card still counts: reading it fails with advice to re-insert it.
            let has_card =
                state == TKSmartCardSlotState::ValidCard || state == TKSmartCardSlotState::MuteCard;
            readers.push(Reader { name, has_card });
        }
        Ok(readers)
    }

    async fn connect(&mut self, reader: &str) -> Result<CryptoTokenKitCard, Error> {
        let slot = self
            .slot(reader)?
            .ok_or_else(|| Error::Transport("the reader is gone".into()))?;
        // SAFETY: `slot` is a live slot; nil (no usable card) becomes `None`.
        let card = unsafe { slot.makeSmartCard() }
            .ok_or_else(|| Error::Transport("no card in the reader".into()))?;
        // SAFETY: `slot` is a live slot; `bytes` returns an NSData we only copy from.
        let atr = unsafe { slot.ATR() }
            .map(|atr| unsafe { atr.bytes() }.to_vec())
            .unwrap_or_default();

        let (tx, rx) = mpsc::channel();
        let reply = RcBlock::new(move |ok: Bool, error: *mut NSError| {
            tx.send(if ok.as_bool() {
                Ok(())
            } else {
                Err(describe(error))
            })
            .ok();
        });
        // SAFETY: `card` is live and the block matches the documented (BOOL, NSError *) signature.
        unsafe { card.beginSessionWithReply(&reply) };
        wait(&rx, "open a session with the card")??;
        Ok(CryptoTokenKitCard { card, atr })
    }

    async fn sleep(&mut self, ms: u32) {
        std::thread::sleep(Duration::from_millis(ms.into()));
    }
}

/// A card with an open session; the session ends when this is dropped.
pub struct CryptoTokenKitCard {
    card: Retained<TKSmartCard>,
    atr: Vec<u8>,
}

impl SmartCard for CryptoTokenKitCard {
    fn atr(&self) -> &[u8] {
        &self.atr
    }

    async fn transmit(&mut self, apdu: &[u8]) -> Result<Vec<u8>, Error> {
        let (tx, rx) = mpsc::channel();
        let reply = RcBlock::new(move |response: *mut NSData, error: *mut NSError| {
            // SAFETY: CryptoTokenKit passes either nil or a valid NSData that lives for this call.
            let response = unsafe { response.as_ref() }.map(NSData::to_vec);
            tx.send(response.ok_or_else(|| describe(error))).ok();
        });
        // SAFETY: the session is open, and the block matches the documented (NSData *, NSError *) signature.
        unsafe { self.card.transmitRequest_reply(&NSData::with_bytes(apdu), &reply) };
        wait(&rx, "exchange data with the card")?
    }
}

impl Drop for CryptoTokenKitCard {
    fn drop(&mut self) {
        // SAFETY: `card` is live; ending a session that is no longer open is harmless.
        unsafe { self.card.endSession() };
    }
}

fn wait<T>(rx: &mpsc::Receiver<Result<T, String>>, what: &str) -> Result<Result<T, Error>, Error> {
    match rx.recv_timeout(REPLY_TIMEOUT) {
        Ok(result) => Ok(result.map_err(|e| Error::Transport(format!("could not {what}: {e}")))),
        Err(_) => Err(Error::Transport(format!(
            "the reader did not answer when trying to {what}"
        ))),
    }
}

/// The error's description, or a generic one when CryptoTokenKit gave none.
fn describe(error: *mut NSError) -> String {
    // SAFETY: CryptoTokenKit passes either nil or a valid NSError that lives for this call.
    unsafe { error.as_ref() }.map_or_else(
        || "unknown error".into(),
        |e| e.localizedDescription().to_string(),
    )
}
