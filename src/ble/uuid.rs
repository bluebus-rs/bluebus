use super::{Error, Result};

/// The part of the Bluetooth Base UUID that follows a 16/32-bit short UUID:
/// `0000xxxx-0000-1000-8000-00805f9b34fb`.
///
/// ```
/// let battery = format!("00002a19{}", bluebus::ble::BASE_UUID_SUFFIX);
/// assert_eq!(battery, bluebus::ble::uuid_from_u16(0x2a19));
/// ```
pub const BASE_UUID_SUFFIX: &str = "-0000-1000-8000-00805f9b34fb";

/// Normalises a Bluetooth UUID to its lower-case 128-bit form, the form
/// BlueZ reports.
///
/// Accepts full UUIDs (with or without dashes) and 16/32-bit short forms,
/// optionally prefixed with `0x`; case-insensitive. Short forms are expanded
/// with the Bluetooth Base UUID.
///
/// ```
/// use bluebus::ble::normalize_uuid;
/// assert_eq!(normalize_uuid("2A19").unwrap(), "00002a19-0000-1000-8000-00805f9b34fb");
/// assert_eq!(normalize_uuid("0x180f").unwrap(), "0000180f-0000-1000-8000-00805f9b34fb");
/// assert_eq!(normalize_uuid("0000FEAA").unwrap(), "0000feaa-0000-1000-8000-00805f9b34fb");
/// assert_eq!(
///     normalize_uuid("B1EB05E0-0000-4000-8000-00000000E2E0").unwrap(),
///     "b1eb05e0-0000-4000-8000-00000000e2e0"
/// );
/// assert!(normalize_uuid("battery").is_err());
/// ```
///
/// # Errors
/// [`Error::InvalidArgument`] if the text is not a UUID.
pub fn normalize_uuid(uuid: &str) -> Result<String> {
    let trimmed = uuid.trim();
    let lower = trimmed.to_ascii_lowercase();
    let (short, hex) = match lower.strip_prefix("0x") {
        Some(rest) => (true, rest),
        None => (false, lower.as_str()),
    };
    let all_hex = |s: &str| s.bytes().all(|b| b.is_ascii_hexdigit());
    let invalid = || Error::InvalidArgument(format!("{uuid:?} is not a Bluetooth UUID"));
    match hex.len() {
        4 if all_hex(hex) => Ok(format!("0000{hex}{BASE_UUID_SUFFIX}")),
        8 if all_hex(hex) => Ok(format!("{hex}{BASE_UUID_SUFFIX}")),
        _ if short => Err(invalid()),
        32 if all_hex(hex) => Ok(format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        )),
        36 => {
            let well_formed = hex.char_indices().all(|(i, c)| match i {
                8 | 13 | 18 | 23 => c == '-',
                _ => c.is_ascii_hexdigit(),
            });
            if well_formed {
                Ok(hex.to_owned())
            } else {
                Err(invalid())
            }
        }
        _ => Err(invalid()),
    }
}

/// Expands a 16-bit assigned number to a 128-bit UUID.
///
/// ```
/// assert_eq!(bluebus::ble::uuid_from_u16(0x2A19), "00002a19-0000-1000-8000-00805f9b34fb");
/// ```
pub fn uuid_from_u16(uuid: u16) -> String {
    format!("0000{uuid:04x}{BASE_UUID_SUFFIX}")
}

/// Expands a 32-bit assigned number to a 128-bit UUID.
///
/// ```
/// assert_eq!(bluebus::ble::uuid_from_u32(0x1234_5678), "12345678-0000-1000-8000-00805f9b34fb");
/// ```
pub fn uuid_from_u32(uuid: u32) -> String {
    format!("{uuid:08x}{BASE_UUID_SUFFIX}")
}

/// Normalises a MAC address to upper-case `AA:BB:CC:DD:EE:FF`. Dashes and
/// underscores are accepted as separators.
pub(crate) fn normalize_address(address: &str) -> Result<String> {
    let parts: Vec<&str> = address.trim().split([':', '-', '_']).collect();
    let ok = parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()));
    if ok {
        Ok(parts.join(":").to_ascii_uppercase())
    } else {
        Err(Error::InvalidArgument(format!(
            "{address:?} is not a Bluetooth address (expected AA:BB:CC:DD:EE:FF)"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_and_long_forms() {
        let battery = "00002a19-0000-1000-8000-00805f9b34fb";
        for input in [
            "2a19",
            "2A19",
            "0x2a19",
            "0X2A19",
            " 2a19 ",
            "00002a19",
            "0x00002A19",
            "00002A19-0000-1000-8000-00805F9B34FB",
            "00002a1900001000800000805f9b34fb",
        ] {
            assert_eq!(normalize_uuid(input).unwrap(), battery, "{input}");
        }
    }

    #[test]
    fn rejects_garbage() {
        for input in [
            "",
            "0x",
            "2a1",
            "2a19f",
            "zz19",
            "0x00002a19-0000-1000-8000-00805f9b34fb",
            "00002a19+0000-1000-8000-00805f9b34fb",
            "00002a19-0000-1000-8000-00805f9b34fg",
            "0x00002a1900001000800000805f9b34fb",
        ] {
            assert!(
                matches!(normalize_uuid(input), Err(Error::InvalidArgument(_))),
                "{input:?}"
            );
        }
    }

    #[test]
    fn from_numbers() {
        assert_eq!(uuid_from_u16(0x180F), normalize_uuid("180f").unwrap());
        assert_eq!(uuid_from_u32(0xFEAA), normalize_uuid("feaa").unwrap());
    }

    #[test]
    fn addresses() {
        assert_eq!(
            normalize_address("aa:bb:cc:dd:ee:0f").unwrap(),
            "AA:BB:CC:DD:EE:0F"
        );
        assert_eq!(
            normalize_address("aa-bb-cc-dd-ee-0f").unwrap(),
            "AA:BB:CC:DD:EE:0F"
        );
        assert!(normalize_address("aa:bb:cc:dd:ee").is_err());
        assert!(normalize_address("aa:bb:cc:dd:ee:gg").is_err());
    }
}
