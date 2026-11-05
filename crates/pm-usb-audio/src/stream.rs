//! Which playback setting (interface and alternate setting) plays a PCM format exactly, and how
//! samples go into its subslots.

use crate::{Endpoint, Rates, Stream, UsbAudioDevice, COMMON_RATES};

/// What the exclusive driver plays a format with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting {
    pub interface: u8,
    pub alt: u8,
    pub channels: u8,
    /// Bytes per sample on the wire.
    pub subslot_bytes: u8,
    /// The setting's bit resolution.
    pub bits: u8,
    pub endpoint: Endpoint,
    /// UAC2: the clock source whose rate is set.
    pub clock: Option<u8>,
}

impl Setting {
    /// Bytes per frame on the wire.
    pub fn frame_bytes(&self) -> usize {
        usize::from(self.channels) * usize::from(self.subslot_bytes)
    }
}

/// The rates a setting takes: UAC1 from its descriptor, UAC2 from the clock (`clock_rates`, what
/// its `RANGE` request answered).
pub fn rates_of(s: &Stream, clock_rates: &[u32]) -> Vec<u32> {
    match &s.rates {
        Rates::Discrete(v) => v.clone(),
        Rates::Continuous { min, max } => COMMON_RATES
            .iter()
            .copied()
            .filter(|r| r >= min && r <= max)
            .collect(),
        Rates::FromClock(_) => clock_rates.to_vec(),
    }
}

/// Integer PCM settings with a data endpoint, whose subslot holds their bits.
fn pcm(dev: &UsbAudioDevice) -> impl Iterator<Item = &Stream> {
    dev.streams.iter().filter(|s| {
        s.pcm
            && !s.float
            && !s.dsd_raw
            && s.endpoint.is_some()
            && (1..=4).contains(&s.subslot_bytes)
            && s.bits > 0
            && s.bits <= s.subslot_bytes * 8
    })
}

/// The setting that plays PCM `rate` / `bits` / `channels` with every sample unchanged: the same
/// channels, the rate listed, at least `bits` bits; the smallest such (16 bit in a 16-bit setting,
/// 24 bit in a 24- or 32-bit one, left-justified). `Err` says why none does.
pub fn choose(
    dev: &UsbAudioDevice,
    clock_rates: &[u32],
    rate: u32,
    bits: u8,
    channels: u8,
) -> Result<Setting, String> {
    let fits = |s: &&Stream| s.channels == channels && s.bits >= bits;
    let best = pcm(dev)
        .filter(fits)
        .filter(|s| rates_of(s, clock_rates).contains(&rate))
        .min_by_key(|s| (s.subslot_bytes, s.bits));
    let Some(s) = best else {
        let why = if pcm(dev).any(|s| s.channels == channels) {
            if pcm(dev)
                .filter(|s| s.channels == channels)
                .any(|s| s.bits >= bits)
            {
                format!("the DAC does not take {} Hz", rate)
            } else {
                format!("the DAC takes fewer than {bits} bits")
            }
        } else {
            format!("the DAC has no {channels}-channel PCM setting")
        };
        return Err(why);
    };
    let clock = match s.rates {
        Rates::FromClock(id) => Some(id),
        _ => None,
    };
    Ok(Setting {
        interface: s.interface,
        alt: s.alt,
        channels: s.channels,
        subslot_bytes: s.subslot_bytes,
        bits: s.bits,
        endpoint: s.endpoint.clone().expect("filtered on an endpoint"),
        clock,
    })
}

/// Every format (rate, bits, channels) it plays unchanged: each setting's rates with 16, 24 and
/// 32 bit up to its resolution.
pub fn formats(dev: &UsbAudioDevice, clock_rates: &[u32]) -> Vec<(u32, u8, u8)> {
    let mut v: Vec<(u32, u8, u8)> = Vec::new();
    for s in pcm(dev) {
        for rate in rates_of(s, clock_rates) {
            for bits in [16u8, 24, 32] {
                if bits <= s.bits && !v.contains(&(rate, bits, s.channels)) {
                    v.push((rate, bits, s.channels));
                }
            }
        }
    }
    v.sort_unstable();
    v
}

/// `samples` (right-aligned `bits`-bit values) into subslots of `subslot_bytes`, little-endian,
/// left-justified (the most significant bit first, the rest zero): values unchanged.
pub fn pack(bits: u8, subslot_bytes: u8, samples: &[i32], out: &mut Vec<u8>) {
    let width = u32::from(subslot_bytes.clamp(1, 4)) * 8;
    let shift = width.saturating_sub(u32::from(bits));
    let n = usize::from(subslot_bytes.clamp(1, 4));
    out.reserve(samples.len() * n);
    for s in samples {
        let v = s.wrapping_shl(shift);
        out.extend_from_slice(&v.to_le_bytes()[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Sync, Uac};

    fn st(alt: u8, subslot: u8, bits: u8, rates: Rates) -> Stream {
        Stream {
            interface: 1,
            alt,
            channels: 2,
            subslot_bytes: subslot,
            bits,
            pcm: true,
            float: false,
            dsd_raw: false,
            rates,
            endpoint: Some(Endpoint {
                address: 0x01,
                sync: Sync::Async,
                max_packet: 0x308,
                interval: 1,
                feedback: Some(0x81),
            }),
        }
    }

    fn dev(streams: Vec<Stream>) -> UsbAudioDevice {
        UsbAudioDevice {
            vendor_id: 1,
            product_id: 2,
            uac: Some(Uac::V1),
            control_interface: Some(0),
            audio_interfaces: vec![0, 1],
            streams,
            clocks: Vec::new(),
            hardware_volume: None,
        }
    }

    #[test]
    fn the_smallest_setting_that_keeps_every_bit() {
        let d = dev(vec![
            st(1, 4, 24, Rates::Discrete(vec![44_100, 96_000])),
            st(2, 2, 16, Rates::Discrete(vec![44_100])),
            st(
                3,
                3,
                24,
                Rates::Continuous {
                    min: 8_000,
                    max: 48_000,
                },
            ),
        ]);
        assert_eq!(choose(&d, &[], 44_100, 16, 2).unwrap().alt, 2);
        assert_eq!(choose(&d, &[], 44_100, 24, 2).unwrap().alt, 3, "3 bytes before 4");
        assert_eq!(
            choose(&d, &[], 96_000, 16, 2).unwrap().alt,
            1,
            "only alt 1 takes 96 kHz"
        );
        assert_eq!(choose(&d, &[], 48_000, 24, 2).unwrap().alt, 3);
        assert_eq!(
            choose(&d, &[], 96_000, 32, 2).unwrap_err(),
            "the DAC takes fewer than 32 bits"
        );
        assert_eq!(
            choose(&d, &[], 192_000, 16, 2).unwrap_err(),
            "the DAC does not take 192000 Hz"
        );
        assert_eq!(
            choose(&d, &[], 44_100, 16, 6).unwrap_err(),
            "the DAC has no 6-channel PCM setting"
        );
        assert!(formats(&d, &[]).contains(&(48_000, 24, 2)));
        assert!(!formats(&d, &[]).contains(&(96_000, 32, 2)));
    }

    #[test]
    fn samples_are_left_justified_and_unchanged() {
        let mut out = Vec::new();
        pack(24, 4, &[0x12_3456, -1], &mut out);
        assert_eq!(out, [0x00, 0x56, 0x34, 0x12, 0x00, 0xff, 0xff, 0xff]);
        out.clear();
        pack(16, 2, &[0x1234, -2], &mut out);
        assert_eq!(out, [0x34, 0x12, 0xfe, 0xff]);
        out.clear();
        pack(24, 3, &[0x12_3456], &mut out);
        assert_eq!(out, [0x56, 0x34, 0x12]);
        out.clear();
        pack(16, 4, &[0x1234], &mut out);
        assert_eq!(out, [0, 0, 0x34, 0x12]);
        out.clear();
        pack(32, 4, &[i32::MIN], &mut out);
        assert_eq!(out, i32::MIN.to_le_bytes());
    }
}
