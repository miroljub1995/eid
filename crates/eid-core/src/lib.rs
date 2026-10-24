//! Serbian eID: the card protocol and the data model, with no I/O of its own.
//!
//! Each platform reaches the reader its own way (PC/SC on desktop, WebUSB in the browser, ...) and
//! implements two traits: [`SmartCard`] sends APDUs to one inserted card, and [`Platform`] lists
//! readers and connects to them. [`read_eid`] reads a card; [`run`] watches the readers and reports
//! them and a [`Status`] whenever a reader or card comes or goes.

mod apdu;
mod card;
mod model;
mod service;
mod tlv;

pub use card::{SmartCard, read_eid};
pub use model::{DocumentKind, EidDocument, Report, Row, Section, format_date, to_latin};
pub use service::{Command, Event, Platform, Reader, Status, run};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The reader or the connection to it failed (no service, unplugged, card removed, ...).
    #[error("{0}")]
    Transport(String),
    #[error("the card answered {sw:04X} to {step}")]
    Status { step: &'static str, sw: u16 },
    #[error("this is not a supported Serbian eID card (ATR {0})")]
    UnknownCard(String),
    #[error("the card returned malformed data: {0}")]
    Malformed(&'static str),
    /// The platform refuses the app access to the reader, e.g. a browser's USB policy.
    #[error("{0}")]
    NotAllowed(String),
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}
