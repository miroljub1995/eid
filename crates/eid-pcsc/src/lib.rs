//! [`Platform`] over the system PC/SC service: CryptoTokenKit on macOS, WinSCard on Windows,
//! pcsc-lite on Linux. Calls block, so drive it from its own thread (e.g. with `pollster`).

use std::ffi::CString;
use std::time::Duration;

use eid_core::{Error, Platform, Reader, SmartCard};
use pcsc::{Context, Protocols, ReaderState, Scope, ShareMode, State};

#[derive(Default)]
pub struct Pcsc {
    /// Dropped whenever the service goes away (on macOS it does when the last reader is unplugged).
    context: Option<Context>,
    states: Vec<ReaderState>,
}

impl Pcsc {
    pub fn new() -> Self {
        Self::default()
    }

    fn list(&mut self, wait: Duration) -> Result<Vec<Reader>, pcsc::Error> {
        let context = match &self.context {
            Some(context) => context,
            None => self.context.insert(Context::establish(Scope::User)?),
        };
        let names = match context.list_readers_owned() {
            Err(pcsc::Error::NoReadersAvailable) => Vec::new(),
            names => names?,
        };
        self.states
            .retain(|state| names.iter().any(|name| name.as_c_str() == state.name()));
        for name in names {
            if !self.states.iter().any(|state| state.name() == name.as_c_str()) {
                self.states.push(ReaderState::new(name, State::UNAWARE));
            }
        }
        if self.states.is_empty() {
            std::thread::sleep(wait);
            return Ok(Vec::new());
        }
        match context.get_status_change(wait, &mut self.states) {
            Ok(()) | Err(pcsc::Error::Timeout) => {}
            Err(error) => return Err(error),
        }
        Ok(self
            .states
            .iter_mut()
            .map(|state| {
                state.sync_current_state();
                let event = state.event_state();
                Reader {
                    name: state.name().to_string_lossy().into_owned(),
                    // A mute card (in the wrong way round, dirty contacts) still counts: reading it
                    // fails with advice to re-insert it, rather than claiming the reader is empty.
                    has_card: event.contains(State::PRESENT),
                }
            })
            .collect())
    }
}

impl Platform for Pcsc {
    type Card = PcscCard;

    async fn readers(&mut self, wait_ms: u32) -> Result<Vec<Reader>, Error> {
        let wait = Duration::from_millis(wait_ms.into());
        match self.list(wait) {
            Ok(readers) => Ok(readers),
            Err(error) => {
                self.context = None;
                self.states.clear();
                match error {
                    // No reader attached or the service is stopped: there is simply nothing to read yet.
                    // A reader unplugged between listing and waiting is the same: it will just be gone.
                    pcsc::Error::NoService
                    | pcsc::Error::ServiceStopped
                    | pcsc::Error::NoReadersAvailable
                    | pcsc::Error::UnknownReader
                    | pcsc::Error::ReaderUnavailable => {
                        std::thread::sleep(wait);
                        Ok(Vec::new())
                    }
                    error => Err(transport(error)),
                }
            }
        }
    }

    async fn connect(&mut self, reader: &str) -> Result<PcscCard, Error> {
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| Error::Transport("PC/SC is not running".into()))?;
        let name = CString::new(reader).map_err(|_| Error::Transport("bad reader name".into()))?;
        // Exclusive while reading so other software can't select another application mid-read.
        let card = match context.connect(&name, ShareMode::Exclusive, Protocols::ANY) {
            Err(pcsc::Error::SharingViolation) => context.connect(&name, ShareMode::Shared, Protocols::ANY),
            card => card,
        }
        .map_err(transport)?;
        let atr = card.status2_owned().map_err(transport)?.atr().to_vec();
        Ok(PcscCard { card, atr })
    }

    async fn sleep(&mut self, ms: u32) {
        std::thread::sleep(Duration::from_millis(ms.into()));
    }
}

pub struct PcscCard {
    card: pcsc::Card,
    atr: Vec<u8>,
}

impl SmartCard for PcscCard {
    fn atr(&self) -> &[u8] {
        &self.atr
    }

    async fn transmit(&mut self, apdu: &[u8]) -> Result<Vec<u8>, Error> {
        let mut buffer = [0; pcsc::MAX_BUFFER_SIZE_EXTENDED];
        Ok(self.card.transmit(apdu, &mut buffer).map_err(transport)?.to_vec())
    }
}

fn transport(error: pcsc::Error) -> Error {
    Error::Transport(error.to_string())
}
