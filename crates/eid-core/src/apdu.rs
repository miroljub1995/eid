//! ISO 7816-4 short command APDUs used by the eID card.

use crate::Error;

pub(crate) const SW_OK: u16 = 0x9000;
/// "End of file reached before reading Le bytes": the data that came back is still valid.
pub(crate) const SW_END_OF_FILE: u16 = 0x6282;

pub(crate) fn select_aid(aid: &[u8]) -> Vec<u8> {
    with_data([0x00, 0xA4, 0x04, 0x00], aid, None)
}

/// Selects an elementary file by its path from the master file.
pub(crate) fn select_path(path: &[u8], le: u8) -> Vec<u8> {
    with_data([0x00, 0xA4, 0x08, 0x00], path, Some(le))
}

pub(crate) fn read_binary(offset: u16, le: u8) -> Vec<u8> {
    let [hi, lo] = offset.to_be_bytes();
    vec![0x00, 0xB0, hi, lo, le]
}

pub(crate) fn get_response(le: u8) -> Vec<u8> {
    vec![0x00, 0xC0, 0x00, 0x00, le]
}

fn with_data(header: [u8; 4], data: &[u8], le: Option<u8>) -> Vec<u8> {
    let mut apdu = header.to_vec();
    // Every path and AID the card uses is a few bytes long, well inside a short APDU.
    apdu.push(data.len() as u8);
    apdu.extend_from_slice(data);
    apdu.extend(le);
    apdu
}

/// Splits a response APDU into its data and status word.
pub(crate) fn split(response: &[u8]) -> Result<(&[u8], u16), Error> {
    let at = response
        .len()
        .checked_sub(2)
        .ok_or(Error::Malformed("response without a status word"))?;
    let (data, sw) = response.split_at(at);
    let sw = match sw {
        [sw1, sw2] => u16::from_be_bytes([*sw1, *sw2]),
        _ => return Err(Error::Malformed("response without a status word")),
    };
    Ok((data, sw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_commands() {
        assert_eq!(
            select_path(&[0x0F, 0x02], 4),
            [0x00, 0xA4, 0x08, 0x00, 0x02, 0x0F, 0x02, 0x04]
        );
        assert_eq!(select_aid(&[0xAA]), [0x00, 0xA4, 0x04, 0x00, 0x01, 0xAA]);
        assert_eq!(read_binary(0x0104, 0xFF), [0x00, 0xB0, 0x01, 0x04, 0xFF]);
    }

    #[test]
    fn splits_status() {
        assert_eq!(split(&[1, 2, 0x90, 0x00]).unwrap(), (&[1u8, 2][..], 0x9000));
        assert!(split(&[0x90]).is_err());
    }
}
