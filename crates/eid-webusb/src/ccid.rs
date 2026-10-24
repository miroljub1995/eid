//! CCID messages (USB CCID spec rev 1.1): plain bytes in and out, so it is tested on the host.

pub const ICC_POWER_ON: u8 = 0x62;
pub const GET_SLOT_STATUS: u8 = 0x65;
pub const XFR_BLOCK: u8 = 0x6F;
pub const DATA_BLOCK: u8 = 0x80;
pub const SLOT_STATUS: u8 = 0x81;

pub const HEADER_LEN: usize = 10;

/// A PC_to_RDR message for slot 0; `params` are the three type-specific header bytes.
pub fn command(kind: u8, seq: u8, params: [u8; 3], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + data.len());
    out.push(kind);
    out.extend((data.len() as u32).to_le_bytes());
    out.extend([0, seq]);
    out.extend(params);
    out.extend_from_slice(data);
    out
}

/// The header of an RDR_to_PC message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub kind: u8,
    pub len: u32,
    pub seq: u8,
    pub status: u8,
    pub error: u8,
}

impl Header {
    pub fn parse(bytes: &[u8]) -> Option<Header> {
        let [kind, l0, l1, l2, l3, _slot, seq, status, error, ..] = *bytes else {
            return None;
        };
        Some(Header {
            kind,
            len: u32::from_le_bytes([l0, l1, l2, l3]),
            seq,
            status,
            error,
        })
    }

    /// The whole message length, header included.
    pub fn total(&self) -> usize {
        HEADER_LEN.saturating_add(self.len as usize)
    }

    pub fn failed(&self) -> bool {
        self.status & 0xC0 == 0x40
    }

    /// The reader asks for more time; the real answer follows.
    pub fn time_extension(&self) -> bool {
        self.status & 0xC0 == 0x80
    }

    /// bmICCStatus 2: no card in the slot.
    pub fn card_absent(&self) -> bool {
        self.status & 0x03 == 2
    }
}

/// Whether the reader exchanges whole APDUs (short or extended), from the CCID class descriptor
/// of interface `interface` in a configuration descriptor. `None` if the descriptor isn't there.
pub fn apdu_level(config: &[u8], interface: u8) -> Option<bool> {
    let mut rest = config;
    let mut in_interface = false;
    while let [len, kind, ..] = *rest {
        let len = usize::from(len);
        let descriptor = rest.get(..len).filter(|_| len >= 2)?;
        match kind {
            4 => in_interface = descriptor.get(2) == Some(&interface),
            0x21 if in_interface => {
                let features = u32::from_le_bytes(descriptor.get(40..44)?.try_into().ok()?);
                return Some(features & 0x0006_0000 != 0);
            }
            _ => {}
        }
        rest = rest.get(len..)?;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_command() {
        assert_eq!(
            command(XFR_BLOCK, 7, [0, 0, 0], &[0x00, 0xA4]),
            [0x6F, 2, 0, 0, 0, 0, 7, 0, 0, 0, 0x00, 0xA4]
        );
    }

    #[test]
    fn parses_a_header() {
        let header = Header::parse(&[0x80, 0x02, 0x01, 0, 0, 0, 9, 0x41, 0xFE, 0, 0xAA]).unwrap();
        assert_eq!(header.len, 258);
        assert_eq!(header.total(), 268);
        assert_eq!(header.seq, 9);
        assert!(header.failed());
        assert!(!header.time_extension());
        assert_eq!(header.error, 0xFE);
        assert!(Header::parse(&[0x80, 0, 0]).is_none());
        assert!(
            Header::parse(&[0x81, 0, 0, 0, 0, 0, 0, 0x82, 0, 0])
                .unwrap()
                .time_extension()
        );
        assert!(
            Header::parse(&[0x81, 0, 0, 0, 0, 0, 0, 0x02, 0, 0])
                .unwrap()
                .card_absent()
        );
    }

    fn config(features: u32) -> Vec<u8> {
        let mut out = vec![9, 2, 0, 0, 1, 1, 0, 0x80, 50]; // configuration
        out.extend([9, 4, 0, 0, 3, 0x0B, 0, 0, 0]); // interface 0, smart card class
        let mut class = vec![0; 54];
        class[0] = 54;
        class[1] = 0x21;
        class[40..44].copy_from_slice(&features.to_le_bytes());
        out.extend(class);
        out.extend([7, 5, 0x81, 2, 64, 0, 0]); // an endpoint
        out
    }

    #[test]
    fn reads_the_exchange_level() {
        assert_eq!(apdu_level(&config(0x0004_0842), 0), Some(true));
        assert_eq!(apdu_level(&config(0x0002_0000), 0), Some(true));
        assert_eq!(apdu_level(&config(0x0001_00BA), 0), Some(false));
        assert_eq!(apdu_level(&config(0x0004_0842), 1), None);
        assert_eq!(apdu_level(&config(0)[..60], 0), None);
        assert_eq!(apdu_level(&[0, 0, 0], 0), None);
    }
}
