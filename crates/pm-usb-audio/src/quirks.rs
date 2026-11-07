//! What some DACs need beyond the spec, by vendor and product id. Each entry says how it was
//! tested; an entry marked untested only records what is expected.

/// One DAC's quirks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Quirks {
    pub name: &'static str,
    /// Sends 16.16 feedback in 4 bytes at full speed (instead of 10.14 in 3).
    pub four_byte_feedback_fs: bool,
    /// Its clock answers `CUR` after a rate change only once the stream runs: the driver doesn't
    /// require the read-back before starting (it still reports `confirmed` only after it).
    pub late_rate_readback: bool,
    /// Never drive its feature unit (some misreport it).
    pub no_hardware_volume: bool,
    /// How the entry was tested.
    pub tested: &'static str,
}

const TABLE: &[(u16, u16, Quirks)] = &[
    (
        0x2972,
        0x0093,
        Quirks {
            name: "FiiO KA17",
            four_byte_feedback_fs: false,
            late_rate_readback: false,
            no_hardware_volume: false,
            tested: "descriptors only (recorded fixture); not yet on a phone",
        },
    ),
    (
        0x05ac,
        0x110a,
        Quirks {
            name: "Apple USB-C to 3.5 mm Headphone Jack Adapter",
            four_byte_feedback_fs: false,
            late_rate_readback: false,
            no_hardware_volume: false,
            tested: "untested: no descriptors recorded yet",
        },
    ),
];

/// The quirks for a DAC (none known: all off).
pub fn lookup(vendor_id: u16, product_id: u16) -> Quirks {
    TABLE
        .iter()
        .find(|(v, p, _)| *v == vendor_id && *p == product_id)
        .map(|(_, _, q)| *q)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dacs_and_the_rest() {
        assert_eq!(lookup(0x2972, 0x0093).name, "FiiO KA17");
        assert!(lookup(0x05ac, 0x110a).tested.starts_with("untested"));
        assert_eq!(lookup(0x1234, 0x5678), Quirks::default());
    }
}
