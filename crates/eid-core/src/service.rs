//! Watching the readers: reads a card when it is inserted and reports what the app should show.

use std::future::Future;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::{EidDocument, Error, SmartCard, read_eid};

/// A card reader the platform can see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reader {
    pub name: String,
    pub has_card: bool,
}

/// How a platform reaches card readers: PC/SC on desktop, WebUSB in the browser, and so on.
pub trait Platform {
    type Card: SmartCard;

    /// Lists the readers, first waiting up to `wait_ms` for a reader or card to come or go. It must
    /// wait when nothing changes: [`run`] calls it in a loop.
    fn readers(&mut self, wait_ms: u32) -> impl Future<Output = Result<Vec<Reader>, Error>>;

    fn connect(&mut self, reader: &str) -> impl Future<Output = Result<Self::Card, Error>>;

    fn sleep(&mut self, ms: u32) -> impl Future<Output = ()>;
}

/// What the app should show.
#[derive(Debug, Clone)]
pub enum Status {
    NoReader,
    NoCard {
        reader: String,
    },
    Reading {
        reader: String,
    },
    Ready {
        reader: String,
        document: Arc<EidDocument>,
    },
    /// Reading the card failed (`reader` is set), or the readers could not be listed.
    Failed {
        reader: Option<String>,
        error: Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Read the inserted card again.
    Reread,
}

const WAIT_MS: u32 = 500;
const RETRY_MS: u32 = 1000;

/// Watches the readers until `commands` is closed, calling `emit` whenever the status changes.
///
/// Uses the first reader that holds a card, or else the first reader. A failed read is retried once
/// on the next poll if a card is still in: it was most likely pulled and pushed back mid-read.
pub async fn run<P: Platform>(mut platform: P, commands: Receiver<Command>, mut emit: impl FnMut(Status)) {
    #[derive(PartialEq)]
    enum Seen {
        Failed(Error),
        NoReader,
        NoCard(String),
        Card(String),
    }
    let mut seen = None;
    let mut retry = false;
    loop {
        let mut reread = std::mem::take(&mut retry);
        loop {
            match commands.try_recv() {
                Ok(Command::Reread) => reread = true,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        let now = match platform.readers(WAIT_MS).await {
            Ok(readers) => match readers.iter().find(|r| r.has_card).or(readers.first()) {
                None => Seen::NoReader,
                Some(Reader {
                    name,
                    has_card: false,
                }) => Seen::NoCard(name.clone()),
                Some(Reader { name, has_card: true }) => Seen::Card(name.clone()),
            },
            Err(error) => Seen::Failed(error),
        };
        if let Seen::Failed(error) = &now {
            if seen.as_ref() != Some(&now) {
                emit(Status::Failed {
                    reader: None,
                    error: error.clone(),
                });
            }
            seen = Some(now);
            platform.sleep(RETRY_MS).await;
            continue;
        }
        if seen.as_ref() == Some(&now) && !reread {
            continue;
        }

        match &now {
            Seen::Failed(_) => {}
            Seen::NoReader => emit(Status::NoReader),
            Seen::NoCard(reader) => emit(Status::NoCard {
                reader: reader.clone(),
            }),
            Seen::Card(reader) => {
                emit(Status::Reading {
                    reader: reader.clone(),
                });
                let read = match platform.connect(reader).await {
                    Ok(mut card) => read_eid(&mut card).await,
                    Err(error) => Err(error),
                };
                emit(match read {
                    Ok(document) => Status::Ready {
                        reader: reader.clone(),
                        document: Arc::new(document),
                    },
                    Err(error) => {
                        // Not for a card we know we can't read: retrying would just fail again.
                        retry = !matches!(error, Error::UnknownCard(_)) && seen.as_ref() != Some(&now);
                        Status::Failed {
                            reader: Some(reader.clone()),
                            error,
                        }
                    }
                });
            }
        }
        seen = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::tests::{FakeCard, sample_card};
    use std::collections::VecDeque;
    use std::sync::mpsc;

    /// Replays a script of reader listings; `commands` is dropped when the script runs out.
    struct Script {
        steps: VecDeque<Vec<Reader>>,
        commands: Option<mpsc::Sender<Command>>,
        reread_at: usize,
        calls: usize,
        /// Connections that fail before the card answers, as when it is pulled mid-read.
        failures: usize,
    }

    impl Platform for Script {
        type Card = FakeCard;

        async fn readers(&mut self, _: u32) -> Result<Vec<Reader>, Error> {
            self.calls += 1;
            if self.calls == self.reread_at
                && let Some(tx) = &self.commands
            {
                tx.send(Command::Reread).ok();
            }
            match self.steps.pop_front() {
                Some(step) => Ok(step),
                None => {
                    self.commands = None;
                    Err(Error::Transport("done".into()))
                }
            }
        }

        async fn connect(&mut self, _: &str) -> Result<FakeCard, Error> {
            if self.failures > 0 {
                self.failures -= 1;
                return Err(Error::Transport("card removed".into()));
            }
            Ok(sample_card())
        }

        async fn sleep(&mut self, _: u32) {}
    }

    fn reader(has_card: bool) -> Reader {
        Reader {
            name: "R".into(),
            has_card,
        }
    }

    #[test]
    fn reports_changes_once_and_reads_inserted_cards() {
        let (tx, rx) = mpsc::channel();
        let steps = VecDeque::from([
            vec![],
            vec![reader(false)],
            vec![reader(false)],
            vec![reader(true)],
            vec![reader(true)],
            vec![reader(true)],
        ]);
        let script = Script {
            steps,
            commands: Some(tx),
            reread_at: 5,
            calls: 0,
            failures: 0,
        };
        let seen = statuses(script, rx);
        let names = names(&seen);
        // The reread command arrives during the fifth listing, so the card is read a second time.
        assert_eq!(
            names,
            [
                "no reader",
                "no card",
                "reading",
                "ready",
                "reading",
                "ready",
                "failed"
            ]
        );
        let Some(Status::Ready { document, .. }) = seen.get(3) else {
            panic!("not ready")
        };
        assert_eq!(document.surname, "ПЕТРОВИЋ");
    }

    fn statuses(script: Script, rx: mpsc::Receiver<Command>) -> Vec<Status> {
        let mut seen = Vec::new();
        pollster::block_on(run(script, rx, |s| seen.push(s)));
        seen
    }

    fn names(seen: &[Status]) -> Vec<&'static str> {
        seen.iter()
            .map(|s| match s {
                Status::NoReader => "no reader",
                Status::NoCard { .. } => "no card",
                Status::Reading { .. } => "reading",
                Status::Ready { .. } => "ready",
                Status::Failed { .. } => "failed",
            })
            .collect()
    }

    #[test]
    fn retries_a_failed_read_once() {
        let (tx, rx) = mpsc::channel();
        let steps = VecDeque::from([vec![reader(true)], vec![reader(true)], vec![reader(true)]]);
        let script = Script {
            steps,
            commands: Some(tx),
            reread_at: 0,
            calls: 0,
            failures: 1,
        };
        assert_eq!(
            names(&statuses(script, rx)),
            ["reading", "failed", "reading", "ready", "failed"]
        );
    }

    #[test]
    fn gives_up_after_the_retry() {
        let (tx, rx) = mpsc::channel();
        let steps = VecDeque::from([vec![reader(true)], vec![reader(true)], vec![reader(true)]]);
        let script = Script {
            steps,
            commands: Some(tx),
            reread_at: 0,
            calls: 0,
            failures: 5,
        };
        assert_eq!(
            names(&statuses(script, rx)),
            ["reading", "failed", "reading", "failed", "failed"]
        );
    }
}
