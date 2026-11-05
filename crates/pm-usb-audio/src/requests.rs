//! The class-specific control requests the exclusive driver sends, as the bytes of a USB setup
//! packet and its data: the UAC2 clock source's sampling frequency (`CS_SAM_FREQ_CONTROL`), the
//! UAC2 feature unit's volume and mute, and UAC1's per-endpoint sampling frequency.

/// One control transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlRequest {
    pub request_type: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    /// What is sent (host to device), or how much is read (device to host).
    pub data: Vec<u8>,
    pub length: u16,
}

impl ControlRequest {
    /// Device to host.
    pub fn reads(&self) -> bool {
        self.request_type & 0x80 != 0
    }
}

/// UAC2 `CUR` and `RANGE`.
pub const CUR: u8 = 0x01;
pub const RANGE: u8 = 0x02;
/// UAC2 clock source control: `CS_SAM_FREQ_CONTROL`.
const CS_SAM_FREQ_CONTROL: u16 = 0x01;
/// UAC2 feature unit controls.
const FU_MUTE_CONTROL: u16 = 0x01;
const FU_VOLUME_CONTROL: u16 = 0x02;
/// Class request to an interface (or an entity in it), host to device / device to host.
const TO_INTERFACE: u8 = 0x21;
const FROM_INTERFACE: u8 = 0xa1;
/// Class request to an endpoint (UAC1 sampling frequency).
const TO_ENDPOINT: u8 = 0x22;
const FROM_ENDPOINT: u8 = 0xa2;
/// UAC1 `SET_CUR` / `GET_CUR`, and its `SAMPLING_FREQ_CONTROL`.
const UAC1_SET_CUR: u8 = 0x01;
const UAC1_GET_CUR: u8 = 0x81;
const UAC1_SAMPLING_FREQ_CONTROL: u16 = 0x01;

fn entity(id: u8, interface: u8) -> u16 {
    (u16::from(id) << 8) | u16::from(interface)
}

/// UAC2: set the clock source `clock` (on AudioControl interface `ac`) to `rate`.
pub fn uac2_set_rate(clock: u8, ac: u8, rate: u32) -> ControlRequest {
    ControlRequest {
        request_type: TO_INTERFACE,
        request: CUR,
        value: CS_SAM_FREQ_CONTROL << 8,
        index: entity(clock, ac),
        data: rate.to_le_bytes().to_vec(),
        length: 4,
    }
}

/// UAC2: read the clock source's current rate (answered by 4 bytes, little-endian).
pub fn uac2_get_rate(clock: u8, ac: u8) -> ControlRequest {
    ControlRequest {
        request_type: FROM_INTERFACE,
        request: CUR,
        value: CS_SAM_FREQ_CONTROL << 8,
        index: entity(clock, ac),
        data: Vec::new(),
        length: 4,
    }
}

/// UAC2: the clock source's rate ranges (answered as [`crate::parse_rate_ranges`] reads).
pub fn uac2_get_rate_ranges(clock: u8, ac: u8) -> ControlRequest {
    ControlRequest {
        request_type: FROM_INTERFACE,
        request: RANGE,
        value: CS_SAM_FREQ_CONTROL << 8,
        index: entity(clock, ac),
        data: Vec::new(),
        // wNumSubRanges and up to 32 sub-ranges of 12 bytes
        length: 2 + 12 * 32,
    }
}

/// UAC2: the feature unit's volume on `channel` (0: master), in 1/256 dB.
pub fn uac2_set_volume(unit: u8, ac: u8, channel: u8, cur: i16) -> ControlRequest {
    ControlRequest {
        request_type: TO_INTERFACE,
        request: CUR,
        value: (FU_VOLUME_CONTROL << 8) | u16::from(channel),
        index: entity(unit, ac),
        data: cur.to_le_bytes().to_vec(),
        length: 2,
    }
}

/// UAC2: the feature unit's current volume on `channel` (2 bytes, 1/256 dB).
pub fn uac2_get_volume(unit: u8, ac: u8, channel: u8) -> ControlRequest {
    ControlRequest {
        request_type: FROM_INTERFACE,
        request: CUR,
        value: (FU_VOLUME_CONTROL << 8) | u16::from(channel),
        index: entity(unit, ac),
        data: Vec::new(),
        length: 2,
    }
}

/// UAC2: the feature unit's volume range on `channel` (wNumSubRanges, then min, max, res).
pub fn uac2_get_volume_range(unit: u8, ac: u8, channel: u8) -> ControlRequest {
    ControlRequest {
        request_type: FROM_INTERFACE,
        request: RANGE,
        value: (FU_VOLUME_CONTROL << 8) | u16::from(channel),
        index: entity(unit, ac),
        data: Vec::new(),
        length: 2 + 6 * 8,
    }
}

/// UAC2: the feature unit's mute on `channel`.
pub fn uac2_set_mute(unit: u8, ac: u8, channel: u8, mute: bool) -> ControlRequest {
    ControlRequest {
        request_type: TO_INTERFACE,
        request: CUR,
        value: (FU_MUTE_CONTROL << 8) | u16::from(channel),
        index: entity(unit, ac),
        data: vec![u8::from(mute)],
        length: 1,
    }
}

/// UAC1: set the sampling frequency of `endpoint` (3 bytes, little-endian).
pub fn uac1_set_rate(endpoint: u8, rate: u32) -> ControlRequest {
    ControlRequest {
        request_type: TO_ENDPOINT,
        request: UAC1_SET_CUR,
        value: UAC1_SAMPLING_FREQ_CONTROL << 8,
        index: u16::from(endpoint),
        data: rate.to_le_bytes()[..3].to_vec(),
        length: 3,
    }
}

/// UAC1: read the sampling frequency of `endpoint`.
pub fn uac1_get_rate(endpoint: u8) -> ControlRequest {
    ControlRequest {
        request_type: FROM_ENDPOINT,
        request: UAC1_GET_CUR,
        value: UAC1_SAMPLING_FREQ_CONTROL << 8,
        index: u16::from(endpoint),
        data: Vec::new(),
        length: 3,
    }
}

/// A rate read back: 4 bytes (UAC2) or 3 (UAC1), little-endian.
pub fn parse_rate(reply: &[u8]) -> Option<u32> {
    match reply.len() {
        3 => Some(u32::from_le_bytes([reply[0], reply[1], reply[2], 0])),
        4.. => Some(u32::from_le_bytes([reply[0], reply[1], reply[2], reply[3]])),
        _ => None,
    }
    .filter(|r| *r > 0)
}

/// A feature unit's volume range (`RANGE` reply): the lowest minimum, the highest maximum and the
/// first resolution, in 1/256 dB.
pub fn parse_volume_range(reply: &[u8]) -> Option<crate::volume::VolumeRange> {
    let n = u16::from_le_bytes([*reply.first()?, *reply.get(1)?]) as usize;
    let mut out: Option<crate::volume::VolumeRange> = None;
    for i in 0..n {
        let at = 2 + i * 6;
        let Some(b) = reply.get(at..at + 6) else { break };
        let min = i16::from_le_bytes([b[0], b[1]]);
        let max = i16::from_le_bytes([b[2], b[3]]);
        let res = u16::from_le_bytes([b[4], b[5]]);
        if min > max {
            continue;
        }
        out = Some(match out {
            None => crate::volume::VolumeRange {
                min,
                max,
                res: res.max(1),
            },
            Some(r) => crate::volume::VolumeRange {
                min: r.min.min(min),
                max: r.max.max(max),
                res: r.res,
            },
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The KA17's clock is entity 0x29 on AudioControl interface 0.
    #[test]
    fn the_setup_packets_are_what_the_spec_says() {
        let r = uac2_set_rate(0x29, 0, 96_000);
        assert_eq!(
            (r.request_type, r.request, r.value, r.index, r.length),
            (0x21, 0x01, 0x0100, 0x2900, 4)
        );
        assert_eq!(r.data, 96_000u32.to_le_bytes());
        let g = uac2_get_rate(0x29, 0);
        assert_eq!(
            (g.request_type, g.request, g.value, g.index, g.length),
            (0xa1, 0x01, 0x0100, 0x2900, 4)
        );
        assert!(g.reads() && !r.reads());
        let rg = uac2_get_rate_ranges(0x29, 0);
        assert_eq!((rg.request_type, rg.request, rg.length), (0xa1, 0x02, 386));
        let v = uac2_set_volume(0x0a, 0, 0, -0x0a00);
        assert_eq!(
            (v.request_type, v.request, v.value, v.index),
            (0x21, 0x01, 0x0200, 0x0a00)
        );
        assert_eq!(v.data, (-0x0a00i16).to_le_bytes());
        let gv = uac2_get_volume(0x0a, 0, 0);
        assert_eq!(
            (gv.request_type, gv.request, gv.value, gv.index, gv.length),
            (0xa1, 0x01, 0x0200, 0x0a00, 2)
        );
        let m = uac2_set_mute(0x0a, 0, 0, true);
        assert_eq!((m.value, m.data.as_slice()), (0x0100, &[1u8][..]));
        let u1 = uac1_set_rate(0x01, 44_100);
        assert_eq!(
            (u1.request_type, u1.request, u1.value, u1.index, u1.length),
            (0x22, 0x01, 0x0100, 0x01, 3)
        );
        assert_eq!(u1.data, [0x44, 0xac, 0x00]);
        let u1g = uac1_get_rate(0x01);
        assert_eq!((u1g.request_type, u1g.request), (0xa2, 0x81));
    }

    #[test]
    fn replies() {
        assert_eq!(parse_rate(&96_000u32.to_le_bytes()), Some(96_000));
        assert_eq!(parse_rate(&[0x44, 0xac, 0x00]), Some(44_100));
        assert_eq!(parse_rate(&[1, 2]), None);
        assert_eq!(parse_rate(&[0, 0, 0, 0]), None);
        // One sub-range: -127 dB .. 0 dB in 1/256 dB, step 1 dB
        let mut b = 1u16.to_le_bytes().to_vec();
        b.extend((-127i16 * 256).to_le_bytes());
        b.extend(0i16.to_le_bytes());
        b.extend(256u16.to_le_bytes());
        assert_eq!(
            parse_volume_range(&b),
            Some(crate::volume::VolumeRange {
                min: -127 * 256,
                max: 0,
                res: 256
            })
        );
        assert_eq!(parse_volume_range(&[]), None);
        assert_eq!(parse_volume_range(&[1, 0, 1]), None, "cut short");
    }
}
