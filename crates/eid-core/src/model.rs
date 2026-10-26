//! What the card holds, and how the official reader presents it.

use crate::{Error, tlv};

/// Which application answered on the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DocumentKind {
    #[default]
    IdCard,
    /// Lična karta za strance.
    ForeignerIdCard,
    /// Dozvola za privremeni boravak i rad.
    ResidencePermit,
}

/// The holder's data as stored on the card: text exactly as read, dates as `DDMMYYYY`.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct EidDocument {
    pub kind: DocumentKind,

    pub doc_reg_no: String,
    pub document_type: String,
    pub document_serial_number: String,
    pub issuing_date: String,
    pub expiry_date: String,
    pub issuing_authority: String,
    pub chip_serial_number: String,
    pub document_name: String,

    pub personal_number: String,
    pub surname: String,
    pub given_name: String,
    pub parent_given_name: String,
    pub sex: String,
    pub place_of_birth: String,
    pub community_of_birth: String,
    pub state_of_birth: String,
    pub state_of_birth_code: String,
    pub date_of_birth: String,
    pub nationality_full: String,
    pub purpose_of_stay: String,
    pub e_note: String,

    pub state: String,
    pub community: String,
    pub place: String,
    pub street: String,
    pub house_number: String,
    pub house_letter: String,
    pub entrance: String,
    pub floor: String,
    pub apartment_number: String,
    pub address_date: String,
    pub address_label: String,

    /// The portrait, a JPEG.
    pub photo: Vec<u8>,
}

type Field = fn(&mut EidDocument) -> &mut String;

const DOCUMENT_TAGS: &[(u16, Field)] = &[
    (1546, |d| &mut d.doc_reg_no),
    (1547, |d| &mut d.document_type),
    (1548, |d| &mut d.document_serial_number),
    (1549, |d| &mut d.issuing_date),
    (1550, |d| &mut d.expiry_date),
    (1551, |d| &mut d.issuing_authority),
    (1681, |d| &mut d.chip_serial_number),
    (1682, |d| &mut d.document_name),
];

const PERSONAL_TAGS: &[(u16, Field)] = &[
    (1558, |d| &mut d.personal_number),
    (1559, |d| &mut d.surname),
    (1560, |d| &mut d.given_name),
    (1561, |d| &mut d.parent_given_name),
    (1562, |d| &mut d.sex),
    (1563, |d| &mut d.place_of_birth),
    (1564, |d| &mut d.community_of_birth),
    (1565, |d| &mut d.state_of_birth),
    (1566, |d| &mut d.date_of_birth),
    (1567, |d| &mut d.state_of_birth_code),
    (1583, |d| &mut d.nationality_full),
    // Older residence permits keep the purpose of stay under 1582; 1683 wins when both are there
    // (see `parse`).
    (1582, |d| &mut d.purpose_of_stay),
    (1683, |d| &mut d.purpose_of_stay),
    (1684, |d| &mut d.e_note),
];

const RESIDENCE_TAGS: &[(u16, Field)] = &[
    (1568, |d| &mut d.state),
    (1569, |d| &mut d.community),
    (1570, |d| &mut d.place),
    (1571, |d| &mut d.street),
    (1572, |d| &mut d.house_number),
    (1573, |d| &mut d.house_letter),
    (1574, |d| &mut d.entrance),
    (1575, |d| &mut d.floor),
    (1578, |d| &mut d.apartment_number),
    (1580, |d| &mut d.address_date),
    (1581, |d| &mut d.address_label),
];

impl EidDocument {
    /// Builds the document from the raw contents of the card's document, personal and residence files.
    pub fn parse(
        kind: DocumentKind,
        document: &[u8],
        personal: &[u8],
        residence: &[u8],
        photo: Vec<u8>,
    ) -> Result<Self, Error> {
        let mut doc = EidDocument {
            kind,
            photo,
            ..Default::default()
        };
        for (file, tags) in [
            (document, DOCUMENT_TAGS),
            (personal, PERSONAL_TAGS),
            (residence, RESIDENCE_TAGS),
        ] {
            for (tag, value) in tlv::parse(file)? {
                let Some((_, field)) = tags.iter().find(|(t, _)| *t == tag) else {
                    continue;
                };
                if value.is_empty() || (tag == 1582 && !doc.purpose_of_stay.is_empty()) {
                    continue;
                }
                *field(&mut doc) = decode(value);
            }
        }
        Ok(doc)
    }

    /// "Mesto, opština, država".
    pub fn place_of_birth_full(&self) -> String {
        join([
            &self.place_of_birth,
            &self.community_of_birth,
            &self.state_of_birth,
        ])
    }

    /// "Mesto, opština, ulica broj/sprat/stan", as the official reader prints it.
    pub fn address_full(&self) -> String {
        let mut street = self.street.clone();
        if !(self.house_number.is_empty() && self.house_letter.is_empty() && self.entrance.is_empty()) {
            street = format!("{street} {}{}", self.house_number, self.house_letter);
            for part in [&self.floor, &self.apartment_number] {
                if !part.is_empty() {
                    street = format!("{street}/{part}");
                }
            }
        }
        join([&self.place, &self.community, &street])
    }

    /// "Given Surname" in Latin script with only the first letters capitalised, e.g. for file names.
    pub fn display_name(&self) -> String {
        [&self.given_name, &self.surname]
            .iter()
            .flat_map(|part| part.split_whitespace())
            .map(|word| {
                // Capitalise each part of a hyphenated name too: "Van Der Berg-Schmidt".
                let mut out = String::new();
                for (i, c) in to_latin(word).to_lowercase().chars().enumerate() {
                    if i == 0 || out.ends_with('-') {
                        out.extend(c.to_uppercase());
                    } else {
                        out.push(c);
                    }
                }
                out
            })
            .collect::<Vec<String>>()
            .join(" ")
    }

    /// The fields in the order and wording of the official reader's print-out.
    pub fn report(&self) -> Report {
        let address_label = if self.address_label.to_lowercase() == "prebivalište" {
            "Prebivalište"
        } else {
            "Prebivalište i adresa stana"
        };
        let row = |label: &'static str, value: &str| Row {
            label,
            value: value.to_owned(),
        };
        let date = |raw: &str| format_date(raw);

        let holder = match self.kind {
            DocumentKind::IdCard => Section {
                title: "Podaci o građaninu",
                rows: vec![
                    row("Prezime", &self.surname),
                    row("Ime", &self.given_name),
                    row("Ime jednog roditelja", &self.parent_given_name),
                    row("Datum rođenja", &date(&self.date_of_birth)),
                    row("Mesto rođenja, opština i država", &self.place_of_birth_full()),
                    row(address_label, &self.address_full()),
                    row("Datum promene adrese", &date(&self.address_date)),
                    row("JMBG", &self.personal_number),
                    row("Pol", &self.sex),
                ],
            },
            DocumentKind::ForeignerIdCard => Section {
                title: "Podaci o strancu",
                rows: vec![
                    row("Prezime", &self.surname),
                    row("Ime", &self.given_name),
                    row("Državljanstvo", &self.nationality_full),
                    row("Datum rođenja", &date(&self.date_of_birth)),
                    row("Osnov boravka", &self.purpose_of_stay),
                    row(address_label, &join([&self.state, &self.address_full()])),
                    row("Datum promene adrese", &date(&self.address_date)),
                    row("Evidencijski broj stranca", &self.personal_number),
                    row("Pol", &self.sex),
                ],
            },
            DocumentKind::ResidencePermit => Section {
                title: "Podaci o strancu",
                rows: vec![
                    row("Prezime", &self.surname),
                    row("Ime", &self.given_name),
                    row("Državljanstvo", &self.nationality_full),
                    row("Datum rođenja", &date(&self.date_of_birth)),
                    row("Mesto rođenja, opština i država", &self.place_of_birth_full()),
                    row(address_label, &self.address_full()),
                    row("Datum promene adrese", &date(&self.address_date)),
                    row("Evidencijski broj stranca", &self.personal_number),
                    row("Pol", &self.sex),
                    row("Osnov boravka", &self.purpose_of_stay),
                    row("Napomena", &self.e_note),
                ],
            },
        };

        let mut document_rows = Vec::new();
        if self.kind == DocumentKind::ResidencePermit {
            document_rows.push(row("Naziv dokumenta", &self.document_name));
        }
        document_rows.extend([
            row("Dokument izdaje", &self.issuing_authority),
            row("Broj dokumenta", &self.doc_reg_no),
            row("Datum izdavanja", &date(&self.issuing_date)),
            row("Važi do", &date(&self.expiry_date)),
        ]);

        let note = match self.kind {
            DocumentKind::IdCard => {
                "U čipu lične karte, podaci o imenu i prezimenu imaoca lične karte ispisani su na nacionalnom \
                 pismu onako kako su ispisani na samom obrascu lične karte, dok su ostali podaci ispisani \
                 latiničkim pismom."
            }
            DocumentKind::ForeignerIdCard => {
                "U čipu lične karte za strance, podaci o imenu i prezimenu stranca ispisani su onako kako su \
                 ispisani na samom obrascu lične karte za stranca latiničnim pismom."
            }
            DocumentKind::ResidencePermit => {
                "U čipu dozvole za privremeni boravak i rad, podaci o imenu i prezimenu imaoca dozvole \
                 ispisani su onako kako su ispisani na samom obrascu dozvole za privremeni boravak \
                 latiničnim pismom."
            }
        };

        Report {
            title: "Čitač elektronske lične karte: štampa podataka",
            sections: vec![
                holder,
                Section {
                    title: "Podaci o dokumentu",
                    rows: document_rows,
                },
            ],
            note,
        }
    }
}

/// Only what identifies the document, so a stray `{:?}` never puts the holder's data in a log.
impl std::fmt::Debug for EidDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EidDocument")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// The print-out: titled sections of labelled values, then a note on how names are written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub title: &'static str,
    pub sections: Vec<Section>,
    pub note: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: &'static str,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: &'static str,
    pub value: String,
}

/// `DDMMYYYY` → `DD.MM.YYYY.`; the card's placeholder dates (year 0xxx) → "Nije dostupan".
pub fn format_date(raw: &str) -> String {
    let b = raw.as_bytes();
    if b.len() != 8 || !b.iter().all(u8::is_ascii_digit) {
        return raw.to_owned();
    }
    if b.get(4) == Some(&b'0') {
        return "Nije dostupan".to_owned();
    }
    // All eight bytes are ASCII digits, so every slice falls on a char boundary.
    format!("{}.{}.{}.", &raw[0..2], &raw[2..4], &raw[4..8])
}

#[rustfmt::skip]
const CYRILLIC: [(char, &str); 30] = [
    ('А', "A"), ('Б', "B"), ('В', "V"), ('Г', "G"), ('Д', "D"), ('Ђ', "Đ"), ('Е', "E"), ('Ж', "Ž"),
    ('З', "Z"), ('И', "I"), ('Ј', "J"), ('К', "K"), ('Л', "L"), ('Љ', "Lj"), ('М', "M"), ('Н', "N"),
    ('Њ', "Nj"), ('О', "O"), ('П', "P"), ('Р', "R"), ('С', "S"), ('Т', "T"), ('Ћ', "Ć"), ('У', "U"),
    ('Ф', "F"), ('Х', "H"), ('Ц', "C"), ('Ч', "Č"), ('Џ', "Dž"), ('Ш', "Š"),
];

/// Serbian Cyrillic → Latin (gajica). Other characters pass through.
pub fn to_latin(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let upper = c.to_uppercase().next().unwrap_or(c);
        match CYRILLIC.iter().find(|(cyr, _)| *cyr == upper) {
            Some((_, latin)) if upper == c => out.push_str(latin),
            Some((_, latin)) => out.push_str(&latin.to_lowercase()),
            None => out.push(c),
        }
    }
    out
}

/// Text on the card is UTF-8; fall back to Latin-1 so a bad byte never hides a whole field.
fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

fn join<'a>(parts: impl IntoIterator<Item = &'a String>) -> String {
    parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> EidDocument {
        EidDocument {
            surname: "ПЕТРОВИЋ".into(),
            given_name: "ЈОВАНА".into(),
            parent_given_name: "MARKO".into(),
            date_of_birth: "15031990".into(),
            place_of_birth: "KRAGUJEVAC".into(),
            community_of_birth: "KRAGUJEVAC".into(),
            state_of_birth: "REPUBLIKA SRBIJA".into(),
            place: "NIŠ".into(),
            community: "NIŠ".into(),
            street: "KNEZA MILOŠA".into(),
            house_number: "12".into(),
            floor: "1".into(),
            apartment_number: "2".into(),
            address_date: "02122022".into(),
            ..Default::default()
        }
    }

    #[test]
    fn formats_like_the_official_print() {
        let doc = sample();
        assert_eq!(
            doc.place_of_birth_full(),
            "KRAGUJEVAC, KRAGUJEVAC, REPUBLIKA SRBIJA"
        );
        assert_eq!(doc.address_full(), "NIŠ, NIŠ, KNEZA MILOŠA 12/1/2");
        assert_eq!(format_date("15031990"), "15.03.1990.");
        assert_eq!(format_date("01010001"), "Nije dostupan");
        assert_eq!(format_date("bad"), "bad");
        assert_eq!(doc.display_name(), "Jovana Petrović");
        let hyphenated = EidDocument {
            given_name: "ANA".into(),
            surname: "BERG-SCHMIDT".into(),
            ..doc.clone()
        };
        assert_eq!(hyphenated.display_name(), "Ana Berg-Schmidt");
        assert!(!format!("{doc:?}").contains("ПЕТРОВИЋ"));
        assert_eq!(to_latin("ЏАК Љиљана"), "DžAK Ljiljana");
    }

    #[test]
    fn address_without_number_is_just_the_street() {
        let doc = EidDocument {
            place: "BEOGRAD".into(),
            street: "BULEVAR".into(),
            ..Default::default()
        };
        assert_eq!(doc.address_full(), "BEOGRAD, BULEVAR");
    }

    #[test]
    fn parses_tags_into_fields() {
        let personal = [
            0x17, 0x06, 0x04, 0x00, b'P', b'E', b'R', b'A', 0x99, 0x09, 0x01, 0x00, b'x',
        ];
        let doc = EidDocument::parse(DocumentKind::IdCard, &[], &personal, &[], vec![]).unwrap();
        assert_eq!(doc.surname, "PERA");
        // 1683 is preferred over 1582 in either order.
        for personal in [
            [0x93, 0x06, 0x01, 0x00, b'N', 0x2E, 0x06, 0x01, 0x00, b'O'],
            [0x2E, 0x06, 0x01, 0x00, b'O', 0x93, 0x06, 0x01, 0x00, b'N'],
        ] {
            let doc = EidDocument::parse(DocumentKind::ResidencePermit, &[], &personal, &[], vec![]).unwrap();
            assert_eq!(doc.purpose_of_stay, "N");
        }
        assert_eq!(decode(&[0xC6]), "Æ");
    }

    #[test]
    fn report_follows_the_official_order() {
        let report = sample().report();
        let labels: Vec<_> = report
            .sections
            .iter()
            .flat_map(|s| s.rows.iter().map(|r| r.label))
            .collect();
        assert_eq!(labels.first(), Some(&"Prezime"));
        assert_eq!(labels.last(), Some(&"Važi do"));
        assert_eq!(labels.len(), 13);
    }
}
