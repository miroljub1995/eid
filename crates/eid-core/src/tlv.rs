//! The card's flat records: a little-endian u16 tag, a little-endian u16 length, then the value.

use crate::Error;

pub(crate) fn parse(mut data: &[u8]) -> Result<Vec<(u16, &[u8])>, Error> {
    let mut records = Vec::new();
    while !data.is_empty() {
        let [t0, t1, l0, l1, rest @ ..] = data else {
            return Err(Error::Malformed("truncated record header"));
        };
        let len = usize::from(u16::from_le_bytes([*l0, *l1]));
        let (value, rest) = rest
            .split_at_checked(len)
            .ok_or(Error::Malformed("truncated record"))?;
        records.push((u16::from_le_bytes([*t0, *t1]), value));
        data = rest;
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_records() {
        let data = [0x0A, 0x06, 0x02, 0x00, b'h', b'i', 0x0B, 0x06, 0x00, 0x00];
        assert_eq!(parse(&data).unwrap(), vec![(1546, &b"hi"[..]), (1547, &b""[..])]);
        assert!(parse(&[]).unwrap().is_empty());
    }

    #[test]
    fn rejects_truncation() {
        assert!(parse(&[0x0A, 0x06, 0x05, 0x00, b'h']).is_err());
        assert!(parse(&[0x0A, 0x06, 0x05]).is_err());
    }
}
