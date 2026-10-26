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

/// How a platform reaches card readers: PC/SC on desktop, and so on.
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

/// What [`run`] reports.
#[derive(Debug, Clone)]
pub enum Event {
    /// The readers the platform sees, whenever one comes or goes or a card is inserted or removed.
    Readers(Vec<Reader>),
    Status(Status),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Read the inserted card again.
    Reread,
    /// Use this reader, or `None` to pick automatically: the first one holding a card.
    Select(Option<String>),
}

const WAIT_MS: u32 = 500;
const RETRY_MS: u32 = 1000;

/// Watches the readers until `commands` is closed, calling `emit` whenever the readers or the status
/// change.
///
/// Uses the selected reader while it is plugged in; otherwise the first reader that holds a card, or
/// else the first reader. A failed read is retried once
/// on the next poll if a card is still in: it was most likely pulled and pushed back mid-read.
pub async fn run<P: Platform>(mut platform: P, commands: Receiver<Command>, mut emit: impl FnMut(Event)) {
    #[derive(PartialEq)]
    enum Seen {
        NoReader,
        NoCard(String),
        Card(String),
    }
    let mut seen = None;
    let mut failed: Option<Error> = None;
    let mut retry = false;
    let mut selected: Option<String> = None;
    let mut listed: Option<Vec<Reader>> = None;
    loop {
        let mut reread = std::mem::take(&mut retry);
        loop {
            match commands.try_recv() {
                Ok(Command::Reread) => reread = true,
                Ok(Command::Select(reader)) if reader != selected => {
                    selected = reader;
                    // Report the picked reader afresh, even if it is the one already shown: the UI may
                    // have shown another one in between.
                    seen = None;
                }
                Ok(Command::Select(_)) => {}
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        let readers = match platform.readers(WAIT_MS).await {
            Ok(readers) => readers,
            Err(error) => {
                if failed.as_ref() != Some(&error) {
                    // Whatever was listed before can't be trusted any more.
                    if listed.as_ref().is_some_and(|l| !l.is_empty()) {
                        emit(Event::Readers(Vec::new()));
                        listed = Some(Vec::new());
                    }
                    emit(Event::Status(Status::Failed {
                        reader: None,
                        error: error.clone(),
                    }));
                }
                failed = Some(error);
                // Report the readers afresh once they can be listed again.
                seen = None;
                platform.sleep(RETRY_MS).await;
                continue;
            }
        };
        failed = None;
        let now = match pick(&readers, selected.as_deref()) {
            None => Seen::NoReader,
            Some(Reader {
                name,
                has_card: false,
            }) => Seen::NoCard(name.clone()),
            Some(Reader { name, has_card: true }) => Seen::Card(name.clone()),
        };
        if listed.as_ref() != Some(&readers) {
            emit(Event::Readers(readers.clone()));
            listed = Some(readers);
        }
        if seen.as_ref() == Some(&now) && !reread {
            continue;
        }

        let status = match &now {
            Seen::NoReader => Status::NoReader,
            Seen::NoCard(reader) => Status::NoCard {
                reader: reader.clone(),
            },
            Seen::Card(reader) => {
                emit(Event::Status(Status::Reading {
                    reader: reader.clone(),
                }));
                let read = match platform.connect(reader).await {
                    Ok(mut card) => read_eid(&mut card).await,
                    Err(error) => Err(error),
                };
                match read {
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
                }
            }
        };
        emit(Event::Status(status));
        seen = Some(now);
    }
}

/// The selected reader while it is there, else the first one holding a card, else the first one.
pub fn pick<'a>(readers: &'a [Reader], selected: Option<&str>) -> Option<&'a Reader> {
    selected
        .and_then(|name| readers.iter().find(|r| r.name == name))
        .or_else(|| readers.iter().find(|r| r.has_card))
        .or_else(|| readers.first())
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
        /// Sends `send` while answering this listing (1-based).
        reread_at: usize,
        send: Vec<Command>,
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
                for command in &self.send {
                    tx.send(command.clone()).ok();
                }
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
            send: vec![Command::Reread],
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
        pollster::block_on(run(script, rx, |e| {
            if let Event::Status(s) = e {
                seen.push(s);
            }
        }));
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
            send: vec![Command::Reread],
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
            send: vec![Command::Reread],
            calls: 0,
            failures: 5,
        };
        assert_eq!(
            names(&statuses(script, rx)),
            ["reading", "failed", "reading", "failed", "failed"]
        );
    }

    fn named(name: &str, has_card: bool) -> Reader {
        Reader {
            name: name.into(),
            has_card,
        }
    }

    #[test]
    fn reads_the_selected_reader() {
        let (tx, rx) = mpsc::channel();
        let both = vec![named("A", true), named("B", true)];
        let steps = VecDeque::from([both.clone(), both.clone(), both.clone(), vec![named("A", true)]]);
        let send = vec![Command::Select(Some("B".into()))];
        let script = Script {
            steps,
            commands: Some(tx),
            reread_at: 1,
            send,
            calls: 0,
            failures: 0,
        };
        let mut events = Vec::new();
        pollster::block_on(run(script, rx, |e| events.push(e)));
        let read: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Status(Status::Ready { reader, .. }) => Some(reader.as_str()),
                _ => None,
            })
            .collect();
        // Automatic picks A first; the selection then switches to B; when B is unplugged it falls back to
        // A, which is read again because it is a different reader from the last one shown.
        assert_eq!(read, ["A", "B", "A"]);
        let lists = events.iter().filter(|e| matches!(e, Event::Readers(_))).count();
        // [A, B], then [A], then nothing once the script ends with a listing error.
        assert_eq!(lists, 3, "the reader list is reported only when it changes");
    }

    #[test]
    fn reports_the_shown_reader_again_after_a_round_trip_selection() {
        // B is picked and the picker set back to automatic within one poll: the watcher ends up on A,
        // which it already showed, but the UI showed B meanwhile, so A must be reported again.
        let (tx, rx) = mpsc::channel();
        let both = vec![named("A", true), named("B", true)];
        let steps = VecDeque::from([both.clone(), both.clone(), both]);
        let send = vec![Command::Select(Some("B".into())), Command::Select(None)];
        let script = Script {
            steps,
            commands: Some(tx),
            reread_at: 1,
            send,
            calls: 0,
            failures: 0,
        };
        assert_eq!(
            names(&statuses(script, rx)),
            ["reading", "ready", "reading", "ready", "failed"]
        );
    }
}
