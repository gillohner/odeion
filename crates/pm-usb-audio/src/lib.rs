//! USB Audio Class (UAC1 and UAC2) devices, from their descriptors: what a USB DAC takes, so the
//! Android app can show "FiiO KA17 · PCM up to 768 kHz / 32 bit · DSD" (STEWARD-ANDROID-USB-DAC.md
//! step 1), and later drive it itself (step 3).
//!
//! The input is what a USB host reads: the device descriptor followed by the configuration
//! descriptors, as Android's `UsbDeviceConnection.getRawDescriptors()` returns them. UAC2 devices
//! keep their sample rates behind the clock source's `RANGE` request; its reply is parsed by
//! [`parse_rate_ranges`].
//!
//! The exclusive driver (step 3) is here too: [`stream`] picks the playback setting for a format,
//! [`requests`] builds the clock and volume requests, [`schedule`] sizes the isochronous packets
//! from the DAC's feedback, [`volume`] keeps the hand-over never louder, [`quirks`] lists what some
//! DACs need, and [`driver`] runs it all on a [`driver::Transport`]. Only [`usbfs`] (Linux and
//! Android) does I/O; everything else runs and is tested on any machine, against a fake DAC.
//!
//! Hostile input is expected: every length is checked, unknown descriptors are skipped, and a
//! malformed one ends the walk with what was understood so far.

// Only `usbfs` (the kernel's USB interface on the file descriptor Android hands over) is unsafe.
#![deny(unsafe_code)]

use serde::Serialize;

pub mod driver;
pub mod quirks;
pub mod requests;
pub mod schedule;
pub mod stream;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod usbfs;
pub mod volume;

/// The USB Audio Class version of a device's audio function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Uac {
    /// USB Audio 1.0 (rates in the format descriptor).
    V1,
    /// USB Audio 2.0 (rates from the clock source).
    V2,
    /// USB Audio 3.0 (described as unsupported for now).
    V3,
}

/// How an isochronous endpoint keeps time with the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Sync {
    None,
    /// The device sets the pace and reports it on a feedback endpoint.
    Async,
    /// The device follows the data rate.
    Adaptive,
    /// Locked to the bus's start-of-frame.
    Sync,
}

/// A data endpoint of a streaming setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Endpoint {
    pub address: u8,
    pub sync: Sync,
    pub max_packet: u16,
    /// Transactions per (micro)frame (high-bandwidth high-speed endpoints: up to 3).
    pub transactions: u8,
    /// UAC1: the endpoint has a sampling frequency control (UAC2: always true, see the clock).
    pub freq_control: bool,
    pub interval: u8,
    /// The feedback endpoint's address (asynchronous devices).
    pub feedback: Option<u8>,
}

/// The sample rates a setting takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Rates {
    /// UAC1: listed in the descriptor.
    Discrete(Vec<u32>),
    /// UAC1: a continuous range.
    Continuous { min: u32, max: u32 },
    /// UAC2: ask the clock source (`RANGE` request on clock `id`), see [`parse_rate_ranges`].
    FromClock(u8),
}

/// One playback setting (an alternate setting of an audio streaming interface).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Stream {
    pub interface: u8,
    pub alt: u8,
    pub channels: u8,
    /// Bytes per sample on the wire.
    pub subslot_bytes: u8,
    /// Significant bits per sample.
    pub bits: u8,
    pub pcm: bool,
    pub float: bool,
    /// Native DSD (UAC2 `TYPE_I_RAW_DATA`).
    pub dsd_raw: bool,
    pub rates: Rates,
    pub endpoint: Option<Endpoint>,
    /// The terminal the setting streams into (AS_GENERAL `bTerminalLink`): an Input Terminal of
    /// type USB streaming, where the playback path starts.
    pub terminal_link: u8,
}

/// A UAC2 clock source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClockSource {
    pub id: u8,
    /// Internal and programmable: its rate can be set.
    pub programmable: bool,
    /// Its frequency control can be written (not only read).
    pub settable: bool,
}

/// One entity of the audio function (a terminal, unit or clock), with what it is connected to:
/// the playback path and its clock are found by following these, not by descriptor order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Entity {
    InputTerminal {
        id: u8,
        terminal_type: u16,
        clock: Option<u8>,
    },
    OutputTerminal {
        id: u8,
        terminal_type: u16,
        source: u8,
    },
    /// A mixer, selector, processing, effect or extension unit.
    Unit {
        id: u8,
        sources: Vec<u8>,
    },
    /// A feature unit: its master channel's volume and mute (`*_settable`: the host may write).
    Feature {
        id: u8,
        source: u8,
        volume: bool,
        mute: bool,
        volume_settable: bool,
        mute_settable: bool,
    },
    ClockSource {
        id: u8,
        settable: bool,
    },
    ClockSelector {
        id: u8,
        sources: Vec<u8>,
    },
    ClockMultiplier {
        id: u8,
        source: u8,
    },
}

/// `wTerminalType` USB streaming: where a playback path starts.
pub const TERMINAL_USB_STREAMING: u16 = 0x0101;

/// A volume (and mute) the device does in hardware: its feature unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HardwareVolume {
    pub unit_id: u8,
    pub volume: bool,
    pub mute: bool,
}

/// A USB audio device's playback side, as its descriptors describe it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsbAudioDevice {
    pub vendor_id: u16,
    pub product_id: u16,
    pub uac: Option<Uac>,
    /// The AudioControl interface's number (requests to the clock and feature unit go there).
    pub control_interface: Option<u8>,
    /// Every interface of its audio function (control, playback, capture), in order: what a
    /// driver taking the DAC over claims and gives back.
    pub audio_interfaces: Vec<u8>,
    /// Playback settings, in the device's order.
    pub streams: Vec<Stream>,
    pub clocks: Vec<ClockSource>,
    pub hardware_volume: Option<HardwareVolume>,
    /// Every terminal, unit and clock of the audio function.
    pub entities: Vec<Entity>,
}

/// A UAC2 clock source's rate range (`RANGE` reply sub-range), in Hz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RateRange {
    pub min: u32,
    pub max: u32,
    pub res: u32,
}

/// The rates players use, for turning a range into a list.
pub const COMMON_RATES: &[u32] = &[
    44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000, 705_600, 768_000,
];

fn u16le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn u24le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        0,
    ]))
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]))
}

/// A UAC2 clock source's `RANGE` reply (layout 3: `wNumSubRanges`, then `dMIN dMAX dRES` per
/// sub-range). Short or malformed replies give what they hold.
pub fn parse_rate_ranges(reply: &[u8]) -> Vec<RateRange> {
    let Some(n) = u16le(reply, 0) else {
        return Vec::new();
    };
    (0..n as usize)
        .map_while(|i| {
            let at = 2 + i * 12;
            Some(RateRange {
                min: u32le(reply, at)?,
                max: u32le(reply, at + 4)?,
                res: u32le(reply, at + 8)?,
            })
        })
        .filter(|r| r.min > 0 && r.min <= r.max)
        .collect()
}

/// The common rates a set of ranges allows.
pub fn rates_in(ranges: &[RateRange]) -> Vec<u32> {
    COMMON_RATES
        .iter()
        .copied()
        .filter(|r| {
            ranges
                .iter()
                .any(|g| *r >= g.min && *r <= g.max && (g.res == 0 || (*r - g.min) % g.res == 0))
        })
        .collect()
}

/// Parse a device's raw descriptors (device descriptor first, then its configurations). The audio
/// function of the first configuration that has one is described.
pub fn parse(raw: &[u8]) -> UsbAudioDevice {
    let mut dev = UsbAudioDevice {
        vendor_id: 0,
        product_id: 0,
        uac: None,
        control_interface: None,
        audio_interfaces: Vec::new(),
        streams: Vec::new(),
        clocks: Vec::new(),
        hardware_volume: None,
        entities: Vec::new(),
    };
    // Where we are: the current interface (number, alt, class, subclass, protocol)
    let mut iface: Option<(u8, u8, u8, u8, u8)> = None;
    let mut configs = 0;
    let mut i = 0;
    while i + 2 <= raw.len() {
        let n = raw[i] as usize;
        if n < 2 || i + n > raw.len() {
            break;
        }
        let d = &raw[i..i + n];
        match d[1] {
            1 if n >= 18 => {
                dev.vendor_id = u16le(d, 8).unwrap_or(0);
                dev.product_id = u16le(d, 10).unwrap_or(0);
            }
            2 => {
                configs += 1;
                // Only the first configuration with audio: the rest repeat it or are alternatives
                if configs > 1 && dev.uac.is_some() {
                    break;
                }
                iface = None;
            }
            4 if n >= 9 => {
                iface = Some((d[2], d[3], d[5], d[6], d[7]));
                if d[5] == 1 && d[6] == 1 && dev.control_interface.is_none() {
                    dev.control_interface = Some(d[2]);
                }
                if d[5] == 1 && !dev.audio_interfaces.contains(&d[2]) {
                    dev.audio_interfaces.push(d[2]);
                }
                if d[5] == 1 && dev.uac.is_none() {
                    dev.uac = Some(match d[7] {
                        0x20 => Uac::V2,
                        0x30 => Uac::V3,
                        _ => Uac::V1,
                    });
                }
            }
            0x24 => {
                if let Some((num, alt, 1, sub, _)) = iface {
                    match sub {
                        1 => control(&mut dev, d),
                        2 => streaming(&mut dev, d, num, alt),
                        _ => {}
                    }
                }
            }
            // CS_ENDPOINT EP_GENERAL (UAC1): whether the endpoint has a sampling frequency control
            0x25 if n >= 4 && d[2] == 0x01 => {
                if let Some((num, alt, 1, 2, _)) = iface {
                    if dev.uac != Some(Uac::V2) && alt != 0 {
                        if let Some(e) = stream_mut(&mut dev, num, alt).endpoint.as_mut() {
                            e.freq_control = d[3] & 0x01 != 0;
                        }
                    }
                }
            }
            5 if n >= 7 => {
                if let Some((num, alt, 1, 2, _)) = iface {
                    endpoint(&mut dev, d, num, alt);
                }
            }
            _ => {}
        }
        i += n;
    }
    // Settings without a data endpoint (UAC1 capture, odd devices) aren't playback
    dev.streams.retain(|s| s.endpoint.is_some());
    dev
}

/// The entity a class-specific AudioControl descriptor describes, when it is one the topology
/// needs.
fn entity(v2: bool, d: &[u8]) -> Option<Entity> {
    let id = *d.get(3)?;
    let sources = |nr_at: usize| -> Option<Vec<u8>> {
        let n = *d.get(nr_at)? as usize;
        d.get(nr_at + 1..nr_at + 1 + n).map(<[u8]>::to_vec)
    };
    Some(match (*d.get(2)?, v2) {
        (0x02, _) => Entity::InputTerminal {
            id,
            terminal_type: u16le(d, 4)?,
            clock: if v2 { Some(*d.get(7)?) } else { None },
        },
        (0x03, _) => Entity::OutputTerminal {
            id,
            terminal_type: u16le(d, 4)?,
            source: *d.get(7)?,
        },
        // MIXER_UNIT, SELECTOR_UNIT: bNrInPins, then the sources
        (0x04 | 0x05, _) => Entity::Unit {
            id,
            sources: sources(4)?,
        },
        (0x06, true) => {
            // bSourceID, bmaControls(0): mute bits 0-1, volume 2-3 (11: the host may write)
            let c = u32le(d, 5)?;
            Entity::Feature {
                id,
                source: *d.get(4)?,
                volume: c & 0x0c != 0,
                mute: c & 0x03 != 0,
                volume_settable: c & 0x0c == 0x0c,
                mute_settable: c & 0x03 == 0x03,
            }
        }
        (0x06, false) => {
            // bSourceID, bControlSize, bmaControls(0): bit 0 mute, bit 1 volume (writable)
            let size = *d.get(5)?;
            let c = match size {
                1 => u32::from(*d.get(6)?),
                2.. => u32::from(u16le(d, 6)?),
                0 => return None,
            };
            Entity::Feature {
                id,
                source: *d.get(4)?,
                volume: c & 0x02 != 0,
                mute: c & 0x01 != 0,
                volume_settable: c & 0x02 != 0,
                mute_settable: c & 0x01 != 0,
            }
        }
        // UAC2 EFFECT_UNIT: wEffectType, bSourceID
        (0x07, true) => Entity::Unit {
            id,
            sources: vec![*d.get(6)?],
        },
        // PROCESSING / EXTENSION units: wType, bNrInPins, then the sources
        (0x08 | 0x09, true) | (0x07 | 0x08, false) => Entity::Unit {
            id,
            sources: sources(6)?,
        },
        (0x0a, true) => Entity::ClockSource {
            id,
            settable: *d.get(5)? & 0x03 == 0x03,
        },
        (0x0b, true) => Entity::ClockSelector {
            id,
            sources: sources(4)?,
        },
        (0x0c, true) => Entity::ClockMultiplier {
            id,
            source: *d.get(4)?,
        },
        _ => return None,
    })
}

/// A class-specific AudioControl descriptor.
fn control(dev: &mut UsbAudioDevice, d: &[u8]) {
    let v2 = dev.uac == Some(Uac::V2);
    if let Some(e) = entity(v2, d) {
        if dev.entities.len() < 256 {
            dev.entities.push(e);
        }
    }
    match d.get(2) {
        // UAC2 CLOCK_SOURCE: bClockID, bmAttributes, bmControls
        Some(0x0a) if v2 && d.len() >= 8 => dev.clocks.push(ClockSource {
            id: d[3],
            programmable: d[4] & 0x03 == 0x03,
            settable: d[5] & 0x03 == 0x03,
        }),
        // FEATURE_UNIT: the master channel's controls
        Some(0x06) => {
            let (unit, volume, mute) = if v2 {
                // bUnitID, bSourceID, bmaControls(0) (4 bytes: mute bits 0-1, volume 2-3)
                let Some(c) = u32le(d, 5) else { return };
                (d[3], c & 0x0c != 0, c & 0x03 != 0)
            } else {
                // bUnitID, bSourceID, bControlSize, bmaControls(0) (bit 0 mute, bit 1 volume)
                let Some(&size) = d.get(5) else { return };
                let c = match size {
                    1 => d.get(6).map(|b| u32::from(*b)),
                    2.. => u16le(d, 6).map(u32::from),
                    0 => None,
                };
                let Some(c) = c else { return };
                (d[3], c & 0x02 != 0, c & 0x01 != 0)
            };
            if (volume || mute) && dev.hardware_volume.is_none() {
                dev.hardware_volume = Some(HardwareVolume {
                    unit_id: unit,
                    volume,
                    mute,
                });
            }
        }
        _ => {}
    }
}

fn stream_mut(dev: &mut UsbAudioDevice, num: u8, alt: u8) -> &mut Stream {
    if let Some(pos) = dev
        .streams
        .iter()
        .position(|s| s.interface == num && s.alt == alt)
    {
        return &mut dev.streams[pos];
    }
    let rates = match (dev.uac, dev.clocks.first()) {
        (Some(Uac::V2), Some(c)) => Rates::FromClock(c.id),
        _ => Rates::Discrete(Vec::new()),
    };
    dev.streams.push(Stream {
        interface: num,
        alt,
        channels: 0,
        subslot_bytes: 0,
        bits: 0,
        pcm: false,
        float: false,
        dsd_raw: false,
        rates,
        endpoint: None,
        terminal_link: 0,
    });
    dev.streams.last_mut().expect("just pushed")
}

/// A class-specific AudioStreaming descriptor.
fn streaming(dev: &mut UsbAudioDevice, d: &[u8], num: u8, alt: u8) {
    if alt == 0 {
        return; // the zero-bandwidth setting
    }
    let v2 = dev.uac == Some(Uac::V2);
    match d.get(2) {
        // AS_GENERAL
        Some(0x01) => {
            let s = stream_mut(dev, num, alt);
            s.terminal_link = d.get(3).copied().unwrap_or(0);
            if v2 {
                // bTerminalLink, bmControls, bFormatType, bmFormats(4), bNrChannels
                let formats = u32le(d, 6).unwrap_or(0);
                s.pcm = formats & 0x01 != 0;
                s.float = formats & 0x04 != 0;
                s.dsd_raw = formats & 0x8000_0000 != 0;
                s.channels = d.get(10).copied().unwrap_or(0);
            } else {
                // bTerminalLink, bDelay, wFormatTag (1 = PCM, 3 = IEEE float)
                let tag = u16le(d, 5).unwrap_or(0);
                s.pcm = tag == 1;
                s.float = tag == 3;
            }
        }
        // FORMAT_TYPE (type I)
        Some(0x02) if d.get(3) == Some(&1) => {
            let s = stream_mut(dev, num, alt);
            if v2 {
                // bSubslotSize, bBitResolution
                s.subslot_bytes = d.get(4).copied().unwrap_or(0);
                s.bits = d.get(5).copied().unwrap_or(0);
            } else {
                // bNrChannels, bSubframeSize, bBitResolution, bSamFreqType, then the rates
                s.channels = d.get(4).copied().unwrap_or(0);
                s.subslot_bytes = d.get(5).copied().unwrap_or(0);
                s.bits = d.get(6).copied().unwrap_or(0);
                let kind = d.get(7).copied().unwrap_or(0);
                s.rates = if kind == 0 {
                    match (u24le(d, 8), u24le(d, 11)) {
                        (Some(min), Some(max)) => Rates::Continuous { min, max },
                        _ => Rates::Discrete(Vec::new()),
                    }
                } else {
                    Rates::Discrete((0..kind as usize).filter_map(|k| u24le(d, 8 + 3 * k)).collect())
                };
            }
        }
        _ => {}
    }
}

/// A standard endpoint descriptor inside a streaming setting.
fn endpoint(dev: &mut UsbAudioDevice, d: &[u8], num: u8, alt: u8) {
    if alt == 0 {
        return;
    }
    let address = d[2];
    let attrs = d[3];
    if attrs & 0x03 != 0x01 {
        return; // not isochronous
    }
    let usage = (attrs >> 4) & 0x03;
    let v2 = dev.uac == Some(Uac::V2);
    let s = stream_mut(dev, num, alt);
    if usage == 1 {
        // A feedback endpoint (device to host)
        if let Some(e) = s.endpoint.as_mut() {
            e.feedback = Some(address);
        }
        return;
    }
    if address & 0x80 != 0 {
        return; // a capture endpoint: not playback
    }
    let sync = match (attrs >> 2) & 0x03 {
        1 => Sync::Async,
        2 => Sync::Adaptive,
        3 => Sync::Sync,
        _ => Sync::None,
    };
    s.endpoint = Some(Endpoint {
        address,
        sync,
        max_packet: u16le(d, 4).unwrap_or(0) & 0x07ff,
        transactions: ((u16le(d, 4).unwrap_or(0) >> 11) & 0x03) as u8 + 1,
        interval: d[6],
        feedback: None,
        // UAC2 has it on the clock; UAC1 says so in its CS endpoint descriptor (just after)
        freq_control: v2,
    });
}

/// What a DAC takes, in a few numbers, for the app's output picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub name: String,
    pub uac: Option<Uac>,
    /// PCM rates, from the descriptors (UAC1) or the clock's ranges (UAC2), ascending.
    pub rates: Vec<u32>,
    pub max_bits: u8,
    /// Native DSD (raw data); DoP isn't visible in descriptors.
    pub dsd: bool,
    pub hardware_volume: bool,
}

impl UsbAudioDevice {
    fn entity(&self, id: u8) -> Option<&Entity> {
        self.entities.iter().find(|e| {
            matches!(e,
                Entity::InputTerminal { id: i, .. } | Entity::OutputTerminal { id: i, .. }
                | Entity::Unit { id: i, .. } | Entity::Feature { id: i, .. }
                | Entity::ClockSource { id: i, .. } | Entity::ClockSelector { id: i, .. }
                | Entity::ClockMultiplier { id: i, .. } if *i == id)
        })
    }

    /// The playback path of a setting streaming into terminal `link`: the entities from the
    /// output terminal back to `link` (output terminal first). `None` when `link` is not a USB
    /// streaming input terminal, or no output or more than one output is reached from it
    /// (ambiguous: nothing is assumed).
    pub fn playback_path(&self, link: u8) -> Option<Vec<u8>> {
        match self.entity(link)? {
            Entity::InputTerminal { terminal_type, .. } if *terminal_type == TERMINAL_USB_STREAMING => {}
            _ => return None,
        }
        let mut found: Vec<Vec<u8>> = Vec::new();
        for e in &self.entities {
            let Entity::OutputTerminal { id, source, .. } = e else {
                continue;
            };
            let mut path = vec![*id];
            if self.walk_back(*source, link, &mut path, 0) {
                found.push(path);
            }
        }
        (found.len() == 1).then(|| found.remove(0))
    }

    /// Follow sources back from `at` to `link`, appending the entities passed.
    fn walk_back(&self, at: u8, link: u8, path: &mut Vec<u8>, depth: usize) -> bool {
        if depth > 32 || path.contains(&at) {
            return false;
        }
        path.push(at);
        if at == link {
            return true;
        }
        let sources: Vec<u8> = match self.entity(at) {
            Some(Entity::Feature { source, .. }) => vec![*source],
            Some(Entity::Unit { sources, .. }) => sources.clone(),
            _ => Vec::new(),
        };
        for s in sources {
            let keep = path.len();
            if self.walk_back(s, link, path, depth + 1) {
                return true;
            }
            path.truncate(keep);
        }
        path.pop();
        false
    }

    /// The feature unit on the playback path from terminal `link` whose volume the host may set
    /// (the one nearest the output); `None` when the path is unknown or has none. A unit on
    /// another path (a microphone's) is never it.
    pub fn playback_unit(&self, link: u8) -> Option<HardwareVolume> {
        self.playback_path(link)?
            .into_iter()
            .find_map(|id| match self.entity(id) {
                Some(Entity::Feature {
                    id,
                    volume_settable: true,
                    mute_settable,
                    ..
                }) => Some(HardwareVolume {
                    unit_id: *id,
                    volume: true,
                    mute: *mute_settable,
                }),
                _ => None,
            })
    }

    /// The clock source driving terminal `link` (UAC2): its `bCSourceID`, through a clock
    /// selector with a single input. `None` when it can't be told for sure (a selector with
    /// several inputs, a multiplier, nothing named): then a rate is never confirmed.
    pub fn playback_clock(&self, link: u8) -> Option<u8> {
        let Some(Entity::InputTerminal {
            clock: Some(mut at), ..
        }) = self.entity(link)
        else {
            return None;
        };
        for _ in 0..8 {
            match self.entity(at)? {
                Entity::ClockSource { id, .. } => return Some(*id),
                Entity::ClockSelector { sources, .. } if sources.len() == 1 => at = sources[0],
                _ => return None,
            }
        }
        None
    }

    /// Whether the clock `id` takes a rate written by the host.
    pub fn clock_settable(&self, id: u8) -> bool {
        matches!(self.entity(id), Some(Entity::ClockSource { settable: true, .. }))
    }

    /// The feature unit Exclusive USB would drive for this device's playback (UAC2: the
    /// playback path of its first PCM setting); `None` for UAC1 (its volume requests are not
    /// spoken) and when the path doesn't say. What a UI calls "its own volume".
    pub fn exclusive_unit(&self) -> Option<HardwareVolume> {
        if self.uac != Some(Uac::V2) {
            return None;
        }
        // A playback setting (an OUT endpoint), never a capture one
        let link = self
            .streams
            .iter()
            .find(|s| s.pcm && s.endpoint.as_ref().is_some_and(|e| e.address & 0x80 == 0))?
            .terminal_link;
        self.playback_unit(link)
    }

    /// The clocks of every playback setting's terminal that can be told for sure (UAC2), each once.
    pub fn playback_clocks(&self) -> Vec<u8> {
        let mut v: Vec<u8> = Vec::new();
        for s in self.streams.iter().filter(|s| s.endpoint.is_some()) {
            if let Some(c) = self.playback_clock(s.terminal_link) {
                if !v.contains(&c) {
                    v.push(c);
                }
            }
        }
        v
    }

    /// Its summary under `name`, with the UAC2 clock's ranges when they were read.
    pub fn summary(&self, name: &str, clock_ranges: &[RateRange]) -> Summary {
        let mut rates: Vec<u32> = Vec::new();
        for s in self.streams.iter().filter(|s| s.pcm) {
            match &s.rates {
                Rates::Discrete(v) => rates.extend(v),
                Rates::Continuous { min, max } => {
                    rates.extend(COMMON_RATES.iter().filter(|r| *r >= min && *r <= max))
                }
                Rates::FromClock(_) => rates.extend(rates_in(clock_ranges)),
            }
        }
        rates.sort_unstable();
        rates.dedup();
        Summary {
            name: name.to_string(),
            uac: self.uac,
            rates,
            max_bits: self
                .streams
                .iter()
                .filter(|s| s.pcm)
                .map(|s| s.bits)
                .max()
                .unwrap_or(0),
            dsd: self.streams.iter().any(|s| s.dsd_raw),
            // Its own volume: the unit on the playback path the app's driver uses (not the first
            // unit in the descriptors, which may be a microphone's)
            hardware_volume: self.exclusive_unit().is_some(),
        }
    }
}

impl Summary {
    /// "FiiO KA17 · PCM up to 768 kHz / 32 bit · DSD · hardware volume".
    pub fn line(&self) -> String {
        let mut parts = vec![self.name.clone()];
        if let Some(max) = self.rates.last() {
            let khz = |r: u32| {
                if r.is_multiple_of(1000) {
                    format!("{}", r / 1000)
                } else {
                    format!("{:.1}", r as f64 / 1000.0)
                }
            };
            parts.push(format!("PCM up to {} kHz / {} bit", khz(*max), self.max_bits));
        }
        if self.dsd {
            parts.push("DSD".into());
        }
        if self.hardware_volume {
            parts.push("hardware volume".into());
        }
        parts.join(" · ")
    }
}
