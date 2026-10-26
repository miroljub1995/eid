//! Reading the eID application from a card.

use std::future::Future;

use crate::apdu::{self, SW_END_OF_FILE, SW_OK};
use crate::{DocumentKind, EidDocument, Error, hex};

/// One inserted card, reached however the platform reaches it (PC/SC, CCID over raw USB, ...).
pub trait SmartCard {
    /// The card's Answer To Reset.
    fn atr(&self) -> &[u8];

    /// Sends a command APDU and returns the whole response APDU, status word included.
    fn transmit(&mut self, apdu: &[u8]) -> impl Future<Output = Result<Vec<u8>, Error>>;
}

/// Cards issued before 18 Aug 2014 (Apollo OS): files are read directly, with a 6-byte header.
const APOLLO_ATR: [u8; 18] = [
    0x3B, 0xB9, 0x18, 0x00, 0x81, 0x31, 0xFE, 0x9E, 0x80, 0x73, 0xFF, 0x61, 0x40, 0x83, 0x00, 0x00, 0x00,
    0xDF,
];

/// Later cards (Gemalto and newer) carry one of these applications, with a 4-byte file header.
const APPLICATIONS: [(DocumentKind, [u8; 11]); 3] = [
    (
        DocumentKind::IdCard,
        [0xF3, 0x81, 0x00, 0x00, 0x02, 0x53, 0x45, 0x52, 0x49, 0x44, 0x01],
    ),
    (
        DocumentKind::ForeignerIdCard,
        [0xF3, 0x81, 0x00, 0x00, 0x02, 0x53, 0x45, 0x52, 0x49, 0x46, 0x01],
    ),
    (
        DocumentKind::ResidencePermit,
        [0xF3, 0x81, 0x00, 0x00, 0x02, 0x53, 0x45, 0x52, 0x52, 0x50, 0x01],
    ),
];

const DOCUMENT_FILE: [u8; 2] = [0x0F, 0x02];
const PERSONAL_FILE: [u8; 2] = [0x0F, 0x03];
const RESIDENCE_FILE: [u8; 2] = [0x0F, 0x04];
const PHOTO_FILE: [u8; 2] = [0x0F, 0x06];

/// The most a short READ BINARY asks for: Le = 0 would mean 256, which some readers mishandle.
const CHUNK: usize = 0xFF;

/// Reads everything the viewer shows: the document, personal and residence files and the photo.
pub async fn read_eid(card: &mut impl SmartCard) -> Result<EidDocument, Error> {
    let (kind, header_len) = if card.atr() == APOLLO_ATR {
        (DocumentKind::IdCard, 6)
    } else {
        (select_application(card).await?, 4)
    };
    let document = read_file(card, &DOCUMENT_FILE, header_len).await?;
    let personal = read_file(card, &PERSONAL_FILE, header_len).await?;
    let residence = read_file(card, &RESIDENCE_FILE, header_len).await?;
    let photo = read_file(card, &PHOTO_FILE, header_len).await?;
    // The photo file wraps the JPEG in one more tag and length.
    let photo = photo.get(4..).unwrap_or_default().to_vec();
    EidDocument::parse(kind, &document, &personal, &residence, photo)
}

async fn select_application(card: &mut impl SmartCard) -> Result<DocumentKind, Error> {
    for (kind, aid) in APPLICATIONS {
        let (_, sw) = exchange(card, &apdu::select_aid(&aid)).await?;
        if sw == SW_OK {
            return Ok(kind);
        }
    }
    Err(Error::UnknownCard(hex(card.atr())))
}

/// Reads an elementary file: its header ends with the content length (u16 LE), the content follows.
async fn read_file(card: &mut impl SmartCard, path: &[u8], header_len: u8) -> Result<Vec<u8>, Error> {
    let (_, sw) = exchange(card, &apdu::select_path(path, 4)).await?;
    if sw != SW_OK {
        return Err(Error::Status {
            step: "select file",
            sw,
        });
    }
    let header = read_binary(card, 0, header_len).await?;
    let (&[.., lo, hi], true) = (header.as_slice(), header.len() == usize::from(header_len)) else {
        return Err(Error::Malformed("short file header"));
    };
    // Apollo cards fill files that were never written with 0xFF.
    if header.iter().all(|&b| b == 0xFF) {
        return Ok(Vec::new());
    }
    let len = usize::from(u16::from_le_bytes([lo, hi]));

    let mut out = Vec::with_capacity(len);
    let mut offset = u16::from(header_len);
    while out.len() < len {
        let want = (len - out.len()).min(CHUNK) as u8;
        let chunk = read_binary(card, offset, want).await?;
        if chunk.is_empty() {
            return Err(Error::Malformed("file shorter than its header says"));
        }
        offset = u16::try_from(chunk.len())
            .ok()
            .and_then(|n| offset.checked_add(n))
            .ok_or(Error::Malformed("file longer than the card can address"))?;
        out.extend_from_slice(&chunk);
    }
    out.truncate(len);
    Ok(out)
}

async fn read_binary(card: &mut impl SmartCard, offset: u16, le: u8) -> Result<Vec<u8>, Error> {
    match exchange(card, &apdu::read_binary(offset, le)).await? {
        (data, SW_OK | SW_END_OF_FILE) => Ok(data),
        (_, sw) => Err(Error::Status {
            step: "read binary",
            sw,
        }),
    }
}

/// Sends a command and follows the T=0 replies that ask for a second step: 6Cxx (resend with
/// Le = xx) and 61xx (xx more bytes waiting, fetch with GET RESPONSE). Returns data and status.
async fn exchange(card: &mut impl SmartCard, command: &[u8]) -> Result<(Vec<u8>, u16), Error> {
    let response = card.transmit(command).await?;
    let (data, mut sw) = apdu::split(&response)?;
    let mut data = data.to_vec();

    // Only a command that ends in Le can be resent with another Le (a short case 2 or case 4 APDU).
    let has_le = match command {
        [_, _, _, _, _] => true,
        [_, _, _, _, lc, rest @ ..] => rest.len() == usize::from(*lc) + 1,
        _ => false,
    };
    if sw >> 8 == 0x6C
        && has_le
        && let [header @ .., _] = command
    {
        let mut resent = header.to_vec();
        resent.push(sw as u8);
        let response = card.transmit(&resent).await?;
        let (more, status) = apdu::split(&response)?;
        data = more.to_vec();
        sw = status;
    }
    // Bounded: no file on the card is anywhere near 64 continuation blocks.
    for _ in 0..64 {
        if sw >> 8 != 0x61 {
            break;
        }
        let response = card.transmit(&apdu::get_response(sw as u8)).await?;
        let (more, status) = apdu::split(&response)?;
        data.extend_from_slice(more);
        sw = status;
    }
    Ok((data, sw))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A Gemalto-style card in memory: answers SELECT and READ BINARY from `files`.
    pub(crate) struct FakeCard {
        pub atr: Vec<u8>,
        pub aid: [u8; 11],
        pub files: HashMap<[u8; 2], Vec<u8>>,
        selected: Option<[u8; 2]>,
        /// Answer READ BINARY with 61xx first, as a T=0 card would.
        pub t0: bool,
        pending: Vec<u8>,
    }

    impl FakeCard {
        pub(crate) fn gemalto(files: HashMap<[u8; 2], Vec<u8>>) -> Self {
            let atr = vec![0x3B, 0xFF, 0x94];
            FakeCard {
                atr,
                aid: APPLICATIONS[0].1,
                files,
                selected: None,
                t0: false,
                pending: vec![],
            }
        }
    }

    fn ok(mut data: Vec<u8>) -> Vec<u8> {
        data.extend([0x90, 0x00]);
        data
    }

    impl SmartCard for FakeCard {
        fn atr(&self) -> &[u8] {
            &self.atr
        }

        async fn transmit(&mut self, apdu: &[u8]) -> Result<Vec<u8>, Error> {
            Ok(match apdu {
                [0x00, 0xA4, 0x04, 0x00, 11, aid @ ..] if aid == self.aid => ok(vec![]),
                [0x00, 0xA4, 0x04, ..] => vec![0x6A, 0x82],
                [0x00, 0xA4, 0x08, 0x00, 2, a, b, 4] if self.files.contains_key(&[*a, *b]) => {
                    self.selected = Some([*a, *b]);
                    ok(vec![])
                }
                [0x00, 0xB0, hi, lo, le] => {
                    let file = self
                        .selected
                        .and_then(|f| self.files.get(&f))
                        .cloned()
                        .unwrap_or_default();
                    let start = usize::from(u16::from_be_bytes([*hi, *lo])).min(file.len());
                    let end = (start + usize::from(*le)).min(file.len());
                    let data = file[start..end].to_vec();
                    if self.t0 {
                        self.pending = data;
                        vec![0x61, self.pending.len() as u8]
                    } else {
                        ok(data)
                    }
                }
                [0x00, 0xC0, 0x00, 0x00, _] => ok(std::mem::take(&mut self.pending)),
                _ => vec![0x6D, 0x00],
            })
        }
    }

    /// A file as the card stores it: a 4-byte header ending in the content length.
    pub(crate) fn file(content: &[u8]) -> Vec<u8> {
        let mut out = vec![0x00, 0x00];
        out.extend((content.len() as u16).to_le_bytes());
        out.extend_from_slice(content);
        out
    }

    pub(crate) fn record(tag: u16, value: &str) -> Vec<u8> {
        let mut out = tag.to_le_bytes().to_vec();
        out.extend((value.len() as u16).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
        out
    }

    pub(crate) fn sample_card() -> FakeCard {
        let photo: Vec<u8> = (0..700u32).map(|i| i as u8).collect();
        let mut photo_file = vec![0x01, 0x02, 0x03, 0x04];
        photo_file.extend(&photo);
        FakeCard::gemalto(HashMap::from([
            (DOCUMENT_FILE, file(&record(1546, "001234567"))),
            (
                PERSONAL_FILE,
                file(&[record(1559, "ПЕТРОВИЋ"), record(1560, "ЈОВАНА")].concat()),
            ),
            (RESIDENCE_FILE, file(&record(1570, "NIŠ"))),
            (PHOTO_FILE, file(&photo_file)),
        ]))
    }

    #[test]
    fn reads_a_gemalto_card() {
        let doc = pollster::block_on(read_eid(&mut sample_card())).unwrap();
        assert_eq!(doc.kind, DocumentKind::IdCard);
        assert_eq!(doc.doc_reg_no, "001234567");
        assert_eq!(doc.surname, "ПЕТРОВИЋ");
        assert_eq!(doc.place, "NIŠ");
        assert_eq!(doc.photo.len(), 700);
        assert_eq!(doc.photo[300], 300u32 as u8);
    }

    #[test]
    fn follows_t0_get_response() {
        let mut card = sample_card();
        card.t0 = true;
        let doc = pollster::block_on(read_eid(&mut card)).unwrap();
        assert_eq!(doc.photo.len(), 700);
    }

    #[test]
    fn finds_the_foreigner_application() {
        let mut card = sample_card();
        card.aid = APPLICATIONS[1].1;
        assert_eq!(
            pollster::block_on(read_eid(&mut card)).unwrap().kind,
            DocumentKind::ForeignerIdCard
        );
    }

    #[test]
    fn rejects_other_cards() {
        let mut card = sample_card();
        card.aid = [0; 11];
        assert!(matches!(
            pollster::block_on(read_eid(&mut card)),
            Err(Error::UnknownCard(_))
        ));
    }

    #[test]
    fn rejects_an_empty_header() {
        let mut card = sample_card();
        card.files.insert(DOCUMENT_FILE, vec![]);
        assert!(matches!(
            pollster::block_on(read_eid(&mut card)),
            Err(Error::Malformed(_))
        ));
    }

    #[test]
    fn rejects_a_file_shorter_than_its_header() {
        let mut card = sample_card();
        card.files
            .insert(DOCUMENT_FILE, vec![0x00, 0x00, 0x10, 0x00, 0x01]);
        assert!(matches!(
            pollster::block_on(read_eid(&mut card)),
            Err(Error::Malformed(_))
        ));
    }
}
