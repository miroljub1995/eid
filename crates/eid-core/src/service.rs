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

    /// Lists the readers, first waiting up to `wait_ms` for a reader or card to come or go.
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
/// Uses the first reader that holds a card, or else the first reader.
pub async fn run<P: Platform>(mut platform: P, commands: Receiver<Command>, mut emit: impl FnMut(Status)) {
    #[derive(PartialEq)]
    enum Seen {
        Nothing,
        Failed(Error),
        NoReader,
        NoCard(String),
        Card(String),
    }
    let mut seen = Seen::Nothing;
    loop {
        let mut reread = false;
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
        if now == seen && !reread {
            if let Seen::Failed(_) = seen {
                platform.sleep(RETRY_MS).await;
            }
            continue;
        }

        match &now {
            Seen::Nothing => {}
            Seen::Failed(error) => emit(Status::Failed {
                reader: None,
                error: error.clone(),
            }),
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
                    Err(error) => Status::Failed {
                        reader: Some(reader.clone()),
                        error,
                    },
                });
            }
        }
        seen = now;
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
        };
        let mut seen = Vec::new();
        pollster::block_on(run(script, rx, |s| seen.push(s)));

        let names: Vec<_> = seen
            .iter()
            .map(|s| match s {
                Status::NoReader => "no reader",
                Status::NoCard { .. } => "no card",
                Status::Reading { .. } => "reading",
                Status::Ready { .. } => "ready",
                Status::Failed { .. } => "failed",
            })
            .collect();
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
}
