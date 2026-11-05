//! Which playback setting (interface and alternate setting) plays a PCM format exactly, and how
//! samples go into its subslots.

use crate::schedule::{nominal_q16, supported, Speed};
use crate::{Endpoint, Rates, Stream, Uac, UsbAudioDevice, COMMON_RATES};

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
    /// UAC2: the clock source driving this setting's terminal (through a single-input selector);
    /// `None` when that can't be told for sure (then nothing is ever confirmed).
    pub clock: Option<u8>,
    /// The rate may be written: UAC2 the clock's frequency control, UAC1 the endpoint's. When not,
    /// the driver only reads it, and plays only when it already is the rate asked.
    pub rate_settable: bool,
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

/// Whether the setting's endpoint carries `rate` at `speed`: an interval the driver plays, and room
/// in each packet for the most frames one carries (the rate per packet rounded up, one more for
/// the DAC's feedback), across all its transactions.
pub fn carries(s: &Stream, rate: u32, speed: Speed) -> bool {
    let Some(ep) = s.endpoint.as_ref() else {
        return false;
    };
    if !supported(speed, ep.interval) {
        return false;
    }
    let per_packet = nominal_q16(rate, speed, ep.interval).div_ceil(1 << 16) + 1;
    let frame = u64::from(s.channels) * u64::from(s.subslot_bytes);
    per_packet * frame <= u64::from(ep.max_packet) * u64::from(ep.transactions.max(1))
}

/// The setting that plays PCM `rate` / `bits` / `channels` with every sample unchanged at `speed`:
/// the same channels, the rate listed and carried by its endpoint, at least `bits` bits; the
/// smallest such (16 bit in a 16-bit setting, 24 bit in a 24- or 32-bit one, left-justified).
/// `Err` says why none does.
pub fn choose(
    dev: &UsbAudioDevice,
    clock_rates: &[u32],
    rate: u32,
    bits: u8,
    channels: u8,
    speed: Speed,
) -> Result<Setting, String> {
    let fits = |s: &&Stream| s.channels == channels && s.bits >= bits;
    let listed = |s: &&Stream| rates_of(s, clock_rates).contains(&rate);
    let best = pcm(dev)
        .filter(fits)
        .filter(listed)
        .filter(|s| carries(s, rate, speed))
        .min_by_key(|s| (s.subslot_bytes, s.bits));
    let Some(s) = best else {
        let why = if pcm(dev).any(|s| s.channels == channels) {
            if pcm(dev).filter(fits).any(|s| listed(&s)) {
                format!("the DAC's USB endpoint can't carry {rate} Hz at this bus speed")
            } else if pcm(dev)
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
    let (clock, rate_settable) = match dev.uac {
        Some(Uac::V2) => {
            let c = dev.playback_clock(s.terminal_link);
            (c, c.is_some_and(|c| dev.clock_settable(c)))
        }
        _ => (None, s.endpoint.as_ref().is_some_and(|e| e.freq_control)),
    };
    Ok(Setting {
        interface: s.interface,
        alt: s.alt,
        channels: s.channels,
        subslot_bytes: s.subslot_bytes,
        bits: s.bits,
        endpoint: s.endpoint.clone().expect("filtered on an endpoint"),
        clock,
        rate_settable,
    })
}

/// Every format (rate, bits, channels) it plays unchanged at `speed`: each setting's rates its
/// endpoint carries, with 16, 24 and 32 bit up to its resolution.
pub fn formats(dev: &UsbAudioDevice, clock_rates: &[u32], speed: Speed) -> Vec<(u32, u8, u8)> {
    let mut v: Vec<(u32, u8, u8)> = Vec::new();
    for s in pcm(dev) {
        for rate in rates_of(s, clock_rates)
            .into_iter()
            .filter(|r| carries(s, *r, speed))
        {
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
            terminal_link: 2,
            rates,
            endpoint: Some(Endpoint {
                address: 0x01,
                sync: Sync::Async,
                max_packet: 0x308,
                transactions: 1,
                freq_control: true,
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
            entities: Vec::new(),
        }
    }

    fn choose_fs(d: &UsbAudioDevice, rate: u32, bits: u8, ch: u8) -> Result<Setting, String> {
        choose(d, &[], rate, bits, ch, Speed::Full)
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
        assert_eq!(choose_fs(&d, 44_100, 16, 2).unwrap().alt, 2);
        assert_eq!(choose_fs(&d, 44_100, 24, 2).unwrap().alt, 3, "3 bytes before 4");
        assert_eq!(
            choose_fs(&d, 96_000, 16, 2).unwrap().alt,
            1,
            "only alt 1 takes 96 kHz"
        );
        assert_eq!(choose_fs(&d, 48_000, 24, 2).unwrap().alt, 3);
        assert_eq!(
            choose_fs(&d, 96_000, 32, 2).unwrap_err(),
            "the DAC takes fewer than 32 bits"
        );
        assert_eq!(
            choose_fs(&d, 192_000, 16, 2).unwrap_err(),
            "the DAC does not take 192000 Hz"
        );
        assert_eq!(
            choose_fs(&d, 44_100, 16, 6).unwrap_err(),
            "the DAC has no 6-channel PCM setting"
        );
        assert!(formats(&d, &[], Speed::Full).contains(&(48_000, 24, 2)));
        assert!(!formats(&d, &[], Speed::Full).contains(&(96_000, 32, 2)));
    }

    /// Codex M (round 1 of #306): a setting whose endpoint can't carry the rate is not chosen,
    /// instead of sending fewer frames than the rate; one that can, is.
    #[test]
    fn a_setting_is_chosen_only_where_its_endpoint_carries_the_rate() {
        let mut small = st(1, 4, 32, Rates::Discrete(vec![48_000, 192_000]));
        small.endpoint.as_mut().unwrap().max_packet = 192; // 24 frames of 8 bytes a packet
        let mut big = st(2, 4, 32, Rates::Discrete(vec![192_000]));
        big.endpoint.as_mut().unwrap().max_packet = 1024;
        let d = dev(vec![small.clone()]);
        // 48 kHz at high speed: 6 (+1) frames a microframe, fits; 192 kHz: 24 (+1), doesn't
        assert_eq!(choose(&d, &[], 48_000, 24, 2, Speed::High).unwrap().alt, 1);
        assert_eq!(
            choose(&d, &[], 192_000, 24, 2, Speed::High).unwrap_err(),
            "the DAC's USB endpoint can't carry 192000 Hz at this bus speed"
        );
        assert!(!formats(&d, &[], Speed::High).contains(&(192_000, 24, 2)));
        let d2 = dev(vec![small, big]);
        assert_eq!(choose(&d2, &[], 192_000, 24, 2, Speed::High).unwrap().alt, 2);
        // Three transactions a microframe carry three times as much
        let mut hb = st(3, 4, 32, Rates::Discrete(vec![768_000]));
        hb.endpoint.as_mut().unwrap().max_packet = 300;
        hb.endpoint.as_mut().unwrap().transactions = 3;
        assert_eq!(
            choose(&dev(vec![hb]), &[], 768_000, 24, 2, Speed::High)
                .unwrap()
                .alt,
            3
        );
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
