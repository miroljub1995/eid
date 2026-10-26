//! The USB CCID protocol that smart card readers speak, for platforms with no smart card service of
//! their own (Android's USB host API, raw USB). A platform supplies [`Bulk`], the reader's two bulk
//! endpoints; [`Ccid`] turns that into card power-up, slot status and APDU exchange.
//!
//! Supports readers that exchange whole APDUs (most current readers); TPDU-level readers are refused.

mod codec;

pub use codec::apdu_level;
use codec::{DATA_BLOCK, GET_SLOT_STATUS, Header, ICC_POWER_OFF, ICC_POWER_ON, SLOT_STATUS, XFR_BLOCK};
use eid_core::Error;

/// A reader's bulk-out and bulk-in endpoints. Both calls block, up to a timeout of the platform's
/// choosing.
pub trait Bulk {
    fn write(&mut self, data: &[u8]) -> Result<(), Error>;
    /// One bulk-in transfer of at most `max` bytes.
    fn read(&mut self, max: usize) -> Result<Vec<u8>, Error>;
}

/// The largest message a reader may send back: an extended APDU response plus the header.
pub const MAX_MESSAGE: usize = 65_538 + codec::HEADER_LEN;

/// How many replies to wait through (stale ones, time extensions) before giving up on the reader.
const MAX_REPLIES: usize = 64;

/// Zero-length reads tolerated in one exchange before the reader counts as gone.
const MAX_EMPTY_READS: usize = 4;

/// One reader's slot 0.
pub struct Ccid<B> {
    bulk: B,
    seq: u8,
}

impl<B: Bulk> Ccid<B> {
    pub fn new(bulk: B) -> Self {
        Ccid { bulk, seq: 0 }
    }

    /// Whether a card is in the slot (powered or not).
    pub fn has_card(&mut self) -> Result<bool, Error> {
        let (header, _) = self.exchange(GET_SLOT_STATUS, &[], SLOT_STATUS)?;
        Ok(!header.card_absent())
    }

    /// Powers the card up, letting the reader pick the voltage; returns its ATR.
    pub fn power_on(&mut self) -> Result<Vec<u8>, Error> {
        Ok(self.exchange(ICC_POWER_ON, &[], DATA_BLOCK)?.1)
    }

    pub fn power_off(&mut self) -> Result<(), Error> {
        self.exchange(ICC_POWER_OFF, &[], SLOT_STATUS).map(|_| ())
    }

    /// Sends a command APDU, returns the response APDU.
    pub fn transmit(&mut self, apdu: &[u8]) -> Result<Vec<u8>, Error> {
        Ok(self.exchange(XFR_BLOCK, apdu, DATA_BLOCK)?.1)
    }

    /// Sends one PC_to_RDR message and returns the reply with the same sequence number.
    fn exchange(&mut self, kind: u8, data: &[u8], reply: u8) -> Result<(Header, Vec<u8>), Error> {
        let seq = self.seq;
        self.seq = seq.wrapping_add(1);
        self.bulk.write(&codec::command(kind, seq, [0; 3], data))?;

        let mut buffer = Vec::new();
        let mut empty = 0;
        for _ in 0..MAX_REPLIES {
            // A message can span several transfers; keep reading until it is whole.
            let header = loop {
                if let Some(header) = Header::parse(&buffer)
                    && buffer.len() >= header.total()
                {
                    break header;
                }
                let chunk = self.bulk.read(MAX_MESSAGE)?;
                // A zero-length packet ends a reply that filled whole transfers; skip a few of them.
                if chunk.is_empty() {
                    empty += 1;
                    if empty > MAX_EMPTY_READS {
                        return Err(Error::Transport("the reader stopped answering".into()));
                    }
                    continue;
                }
                buffer.extend_from_slice(&chunk);
                if Header::parse(&buffer).is_some_and(|h| h.total() > MAX_MESSAGE) {
                    return Err(Error::Transport("the reader sent a malformed reply".into()));
                }
            };
            let message: Vec<u8> = buffer.drain(..header.total()).collect();
            if header.seq != seq || header.time_extension() {
                continue;
            }
            if header.kind != reply {
                return Err(Error::Transport(format!(
                    "the reader sent an unexpected {:02X} reply",
                    header.kind
                )));
            }
            if header.failed() {
                return Err(failure(&header));
            }
            return Ok((
                header,
                message.get(codec::HEADER_LEN..).unwrap_or_default().to_vec(),
            ));
        }
        Err(Error::Transport("the reader never answered".into()))
    }
}

fn failure(header: &Header) -> Error {
    if header.card_absent() {
        return Error::Transport("card removed".into());
    }
    Error::Transport(match header.error {
        0xFE => "the card does not answer (inserted the wrong way round?)".into(),
        0xFB => "the reader reported a hardware error".into(),
        0xFC | 0xFD => "transmission error between reader and card".into(),
        error => format!("the reader reported error {error:02X}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A reader that answers from a script, recording what it was sent.
    #[derive(Default)]
    struct Script {
        sent: Vec<Vec<u8>>,
        replies: VecDeque<Vec<u8>>,
    }

    impl Bulk for &mut Script {
        fn write(&mut self, data: &[u8]) -> Result<(), Error> {
            self.sent.push(data.to_vec());
            Ok(())
        }
        fn read(&mut self, max: usize) -> Result<Vec<u8>, Error> {
            let mut reply = self
                .replies
                .pop_front()
                .ok_or(Error::Transport("timeout".into()))?;
            reply.truncate(max);
            Ok(reply)
        }
    }

    fn reply(kind: u8, seq: u8, status: u8, error: u8, data: &[u8]) -> Vec<u8> {
        let mut out = vec![kind];
        out.extend((data.len() as u32).to_le_bytes());
        out.extend([0, seq, status, error, 0]);
        out.extend_from_slice(data);
        out
    }

    #[test]
    fn exchanges_an_apdu() {
        let mut script = Script::default();
        script
            .replies
            .push_back(reply(DATA_BLOCK, 0, 0, 0, &[0x90, 0x00]));
        assert_eq!(
            Ccid::new(&mut script).transmit(&[0x00, 0xA4]).unwrap(),
            [0x90, 0x00]
        );
        assert_eq!(script.sent, [[0x6F, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0xA4]]);
    }

    #[test]
    fn joins_a_reply_split_across_transfers_and_skips_stale_and_waiting_ones() {
        let mut script = Script::default();
        let data: Vec<u8> = (0..=255).collect();
        let whole = reply(DATA_BLOCK, 0, 0, 0, &data);
        script.replies.extend([
            reply(DATA_BLOCK, 9, 0, 0, &[1]),   // stale, from an earlier exchange
            reply(DATA_BLOCK, 0, 0x80, 1, &[]), // time extension
            whole[..64].to_vec(),
            whole[64..].to_vec(),
        ]);
        assert_eq!(Ccid::new(&mut script).transmit(&[0]).unwrap(), data);
    }

    #[test]
    fn reports_slot_status_and_failures() {
        let mut script = Script::default();
        script.replies.extend([
            reply(SLOT_STATUS, 0, 0x02, 0, &[]),
            reply(SLOT_STATUS, 1, 0x01, 0, &[]),
            reply(DATA_BLOCK, 2, 0x41, 0xFE, &[]),
        ]);
        let mut ccid = Ccid::new(&mut script);
        assert!(!ccid.has_card().unwrap());
        assert!(ccid.has_card().unwrap());
        assert!(matches!(ccid.power_on(), Err(Error::Transport(e)) if e.contains("wrong way")));
    }

    #[test]
    fn skips_zero_length_packets_but_not_forever() {
        let mut script = Script::default();
        script
            .replies
            .extend([vec![], reply(DATA_BLOCK, 0, 0, 0, &[0x90, 0x00])]);
        assert_eq!(Ccid::new(&mut script).transmit(&[0]).unwrap(), [0x90, 0x00]);

        let mut script = Script::default();
        script.replies.extend(std::iter::repeat_n(vec![], 10));
        assert!(
            matches!(Ccid::new(&mut script).transmit(&[0]), Err(Error::Transport(e)) if e.contains("stopped"))
        );
    }

    #[test]
    fn refuses_oversized_and_unexpected_replies() {
        let mut script = Script::default();
        let mut huge = reply(DATA_BLOCK, 0, 0, 0, &[]);
        huge[1..5].copy_from_slice(&u32::MAX.to_le_bytes());
        script.replies.push_back(huge);
        assert!(Ccid::new(&mut script).transmit(&[0]).is_err());

        let mut script = Script::default();
        script.replies.push_back(reply(SLOT_STATUS, 0, 0, 0, &[]));
        assert!(Ccid::new(&mut script).transmit(&[0]).is_err());
    }
}
