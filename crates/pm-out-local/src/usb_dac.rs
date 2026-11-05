//! USB DACs on an Android phone, as the app's Kotlin side reports them
//! (`tools/release/android/UsbAudio.kt`): what is attached, by name, with the rates, sample sizes
//! and channels Android lists for it, and, when the app already has permission for the USB
//! device, its raw descriptors (parsed by [`pm_usb_audio`]).
//!
//! Nothing here talks to the JVM, so it is built and tested on every host: the app hands the
//! report over as text ([`parse_poll`], [`parse_report`]), [`describe`] turns it into the
//! [`OutputDevice`] the phone's output reports, and [`set_current`] keeps it for that output.
//!
//! The phone's output still plays through Android's mixer here ([`DeviceMode::Shared`]): the DAC
//! is described, never claimed bit-perfect.

use pm_zones::{DeviceKind, DeviceMode, DsdKind, OutputDevice, UsbIds};
use serde::Deserialize;
use std::sync::Mutex;

/// `AudioDeviceInfo.TYPE_USB_DEVICE`.
pub const TYPE_USB_DEVICE: i32 = 11;
/// `AudioDeviceInfo.TYPE_USB_HEADSET`.
pub const TYPE_USB_HEADSET: i32 = 22;
/// `AudioFormat.ENCODING_PCM_16BIT`.
pub const ENCODING_PCM_16BIT: i32 = 2;
/// `AudioFormat.ENCODING_PCM_24BIT_PACKED` (API 31).
pub const ENCODING_PCM_24BIT_PACKED: i32 = 21;
/// `AudioFormat.ENCODING_PCM_32BIT` (API 31).
pub const ENCODING_PCM_32BIT: i32 = 22;
/// `AudioFormat.ENCODING_DSD` (API 34).
pub const ENCODING_DSD: i32 = 31;

/// The largest report accepted, in bytes (raw descriptors as hex are a few KB).
pub const MAX_REPORT: usize = 256 * 1024;
/// At most this many devices, rates, encodings or channel counts are read from a report.
const MAX_ITEMS: usize = 64;
/// Raw descriptors larger than this (decoded) are ignored.
const MAX_DESCRIPTORS: usize = 64 * 1024;
const MAX_NAME: usize = 64;

/// What `UsbAudio.kt` reports: Android's SDK level and the USB audio sinks attached.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Report {
    /// The report's format version (1).
    #[serde(default)]
    pub v: u32,
    /// `Build.VERSION.SDK_INT`.
    #[serde(default)]
    pub sdk: u32,
    #[serde(default)]
    pub devices: Vec<ReportDevice>,
}

/// One output `AudioDeviceInfo`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ReportDevice {
    /// `getId()`.
    #[serde(default)]
    pub id: i32,
    /// `getType()`.
    #[serde(default, rename = "type")]
    pub kind: i32,
    /// `getProductName()`.
    #[serde(default)]
    pub name: String,
    /// `getSampleRates()`: empty means any rate.
    #[serde(default)]
    pub rates: Vec<u32>,
    /// `getEncodings()`.
    #[serde(default)]
    pub encodings: Vec<i32>,
    /// `getChannelCounts()`.
    #[serde(default)]
    pub channel_counts: Vec<u32>,
    /// Android routes media there now (`getAudioDevicesForAttributes`, API 33); `None` before.
    #[serde(default)]
    pub routed: Option<bool>,
    /// The matching `UsbDevice`, when one was found.
    #[serde(default)]
    pub usb: Option<ReportUsb>,
}

/// The `UsbDevice` behind an audio device.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ReportUsb {
    #[serde(default)]
    pub vendor_id: u16,
    #[serde(default)]
    pub product_id: u16,
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub product: Option<String>,
    /// `UsbDeviceConnection.getRawDescriptors()` as hex, only when the app already had permission.
    #[serde(default)]
    pub descriptors: Option<String>,
}

/// `UsbAudio.next()`'s answer: `"<version>\n<report>"`.
pub fn parse_poll(s: &str) -> Option<(u64, &str)> {
    let (v, rest) = s.split_once('\n')?;
    Some((v.trim().parse().ok()?, rest))
}

/// A report from its JSON, with every list cut to a sane length.
pub fn parse_report(json: &str) -> Result<Report, String> {
    if json.len() > MAX_REPORT {
        return Err(format!("a USB audio report of {} bytes", json.len()));
    }
    let mut r: Report = serde_json::from_str(json).map_err(|e| format!("a USB audio report: {e}"))?;
    r.devices.truncate(MAX_ITEMS);
    for d in &mut r.devices {
        d.rates.truncate(MAX_ITEMS);
        d.encodings.truncate(MAX_ITEMS);
        d.channel_counts.truncate(MAX_ITEMS);
    }
    Ok(r)
}

fn is_usb_sink(d: &ReportDevice) -> bool {
    d.kind == TYPE_USB_DEVICE || d.kind == TYPE_USB_HEADSET
}

/// Printable, trimmed, at most [`MAX_NAME`] characters.
fn clean(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    s.chars().take(MAX_NAME).collect::<String>().trim().to_string()
}

/// The name people know it by: Android's product name without the "USB-Audio - " some phones put
/// first, else the USB product string, with the maker in front when neither says it.
pub fn display_name(d: &ReportDevice) -> String {
    let mut name = clean(&d.name);
    for prefix in ["USB-Audio - ", "USB Audio - ", "USB-Audio-"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            name = rest.trim().to_string();
        }
    }
    let usb = d.usb.as_ref();
    if name.is_empty() {
        name = usb
            .and_then(|u| u.product.as_deref())
            .map(clean)
            .unwrap_or_default();
    }
    if let Some(maker) = usb.and_then(|u| u.manufacturer.as_deref()).map(clean) {
        if !maker.is_empty() && !name.to_lowercase().contains(&maker.to_lowercase()) {
            name = if name.is_empty() {
                maker
            } else {
                format!("{maker} {name}")
            };
        }
    }
    if name.is_empty() {
        name = "USB audio device".into();
    }
    name.chars().take(MAX_NAME).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) || s.len() / 2 > MAX_DESCRIPTORS {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// The descriptors' summary, when they are there, parse as an audio device, and are the same
/// USB device (vendor and product id).
fn descriptors_summary(d: &ReportDevice, name: &str) -> Option<pm_usb_audio::Summary> {
    let usb = d.usb.as_ref()?;
    let raw = unhex(usb.descriptors.as_deref()?)?;
    let parsed = pm_usb_audio::parse(&raw);
    if parsed.uac.is_none() || (parsed.vendor_id, parsed.product_id) != (usb.vendor_id, usb.product_id) {
        return None;
    }
    Some(parsed.summary(name, &[]))
}

/// The Android major version for an SDK level, for words people know ("Android 13").
fn android_version(sdk: u32) -> String {
    match sdk {
        36.. => (sdk - 20).to_string(),
        35 => "15".into(),
        34 => "14".into(),
        33 => "13".into(),
        32 => "12L".into(),
        31 => "12".into(),
        30 => "11".into(),
        29 => "10".into(),
        28 => "9".into(),
        26 | 27 => "8".into(),
        _ => format!("API {sdk}"),
    }
}

/// The USB DAC the phone plays through, from a report: the USB sink Android routes media to, else
/// the first one. `None` when no USB audio sink is attached.
pub fn describe(report: &Report) -> Option<OutputDevice> {
    let sinks: Vec<&ReportDevice> = report.devices.iter().filter(|d| is_usb_sink(d)).collect();
    let d = *sinks
        .iter()
        .find(|d| d.routed == Some(true))
        .or_else(|| sinks.first())?;
    let name = display_name(d);
    let desc = descriptors_summary(d, &name);
    let mut rates: Vec<u32> = d
        .rates
        .iter()
        .copied()
        .filter(|r| (8_000..=1_536_000).contains(r))
        .collect();
    if rates.is_empty() {
        // Android lists none for "any rate": the descriptors say which
        rates = desc.as_ref().map(|s| s.rates.clone()).unwrap_or_default();
    }
    rates.sort_unstable();
    rates.dedup();
    let mut bits: Vec<u8> = d
        .encodings
        .iter()
        .filter_map(|e| match *e {
            ENCODING_PCM_16BIT => Some(16),
            ENCODING_PCM_24BIT_PACKED => Some(24),
            ENCODING_PCM_32BIT => Some(32),
            _ => None,
        })
        .collect();
    if bits.is_empty() {
        bits.extend(desc.as_ref().map(|s| s.max_bits).filter(|b| *b > 0));
    }
    bits.sort_unstable();
    bits.dedup();
    let channels = d
        .channel_counts
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
        .min(u32::from(u8::MAX)) as u8;
    let dsd = (d.encodings.contains(&ENCODING_DSD) || desc.as_ref().is_some_and(|s| s.dsd))
        .then_some(DsdKind::Native);
    let hardware_volume = desc.as_ref().is_some_and(|s| s.hardware_volume);
    let summary = pm_usb_audio::Summary {
        name: name.clone(),
        uac: desc.as_ref().and_then(|s| s.uac),
        rates: rates.clone(),
        max_bits: bits.last().copied().unwrap_or(0),
        dsd: dsd.is_some(),
        hardware_volume,
    }
    .line();
    let note = if report.sdk == 0 || report.sdk >= 34 {
        None
    } else {
        Some(format!(
            "Bit-perfect playback needs Android 14 or later; this phone has Android {}.",
            android_version(report.sdk)
        ))
    };
    Some(OutputDevice {
        name,
        kind: if d.kind == TYPE_USB_HEADSET {
            DeviceKind::UsbHeadset
        } else {
            DeviceKind::UsbDac
        },
        summary,
        max_rate: rates.last().copied().unwrap_or(0),
        max_bits: bits.last().copied().unwrap_or(0),
        rates,
        bits,
        channels,
        dsd,
        hardware_volume,
        // Android's mixer still plays it: shared, and nothing confirmed
        mode: DeviceMode::Shared,
        modes: vec![DeviceMode::Shared],
        confirmed: false,
        note,
        usb: d
            .usb
            .as_ref()
            .filter(|u| u.vendor_id != 0 || u.product_id != 0)
            .map(|u| UsbIds {
                vendor_id: u.vendor_id,
                product_id: u.product_id,
            }),
    })
}

/// The USB DAC the phone's output plays through now (`None`: the speaker, headphones, Bluetooth).
static CURRENT: Mutex<Option<OutputDevice>> = Mutex::new(None);

/// Keep `d` as the phone's DAC. Returns whether it changed (the app then registers its player
/// again, so the node and the UI learn it).
pub fn set_current(d: Option<OutputDevice>) -> bool {
    let mut cur = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    if *cur == d {
        return false;
    }
    *cur = d;
    true
}

/// The phone's DAC, as last set.
pub fn current() -> Option<OutputDevice> {
    CURRENT.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What UsbAudio.kt reports for a FiiO KA17 on an Android 14 phone (rates and encodings as
    /// Android lists them for the KA17's UAC2 interface).
    const KA17: &str = r#"{"v":1,"sdk":34,"devices":[
        {"id":3,"type":2,"name":"Pixel 8","rates":[48000],"encodings":[2],"channel_counts":[2]},
        {"id":41,"type":11,"name":"FiiO KA17","rates":[44100,48000,88200,96000,176400,192000,352800,384000,705600,768000],
         "encodings":[2,21,22,4],"channel_counts":[2],"routed":true,
         "usb":{"vendor_id":10610,"product_id":147,"manufacturer":"FiiO","product":"FiiO KA17"}}]}"#;

    #[test]
    fn the_ka17_is_described_by_name_with_rates_and_sizes() {
        let d = describe(&parse_report(KA17).unwrap()).unwrap();
        assert_eq!(d.name, "FiiO KA17");
        assert_eq!(d.kind, DeviceKind::UsbDac);
        assert_eq!(d.rates.first(), Some(&44_100));
        assert_eq!((d.max_rate, d.max_bits), (768_000, 32));
        assert_eq!(d.bits, [16, 24, 32]);
        assert_eq!(d.channels, 2);
        assert_eq!(d.summary, "FiiO KA17 · PCM up to 768 kHz / 32 bit");
        assert_eq!(
            d.usb,
            Some(UsbIds {
                vendor_id: 0x2972,
                product_id: 0x0093
            })
        );
        assert_eq!(d.note, None);
    }

    #[test]
    fn it_is_never_called_bit_perfect_while_android_mixes() {
        for sdk in [29, 33, 34, 35] {
            let json = KA17.replace("\"sdk\":34", &format!("\"sdk\":{sdk}"));
            let d = describe(&parse_report(&json).unwrap()).unwrap();
            assert_eq!((d.mode, d.confirmed), (DeviceMode::Shared, false), "sdk {sdk}");
            assert_eq!(d.modes, [DeviceMode::Shared]);
            let text = serde_json::to_string(&d).unwrap();
            assert!(!text.contains("bit_perfect"), "{text}");
        }
        // Below Android 14 the note says why bit-perfect isn't possible
        let d = describe(&parse_report(&KA17.replace("\"sdk\":34", "\"sdk\":33")).unwrap()).unwrap();
        assert_eq!(
            d.note.as_deref(),
            Some("Bit-perfect playback needs Android 14 or later; this phone has Android 13.")
        );
    }

    #[test]
    fn the_wire_shape_is_what_the_ui_reads() {
        let d = describe(&parse_report(KA17).unwrap()).unwrap();
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["kind"], "usb_dac");
        assert_eq!(v["mode"], "shared");
        assert_eq!(v["modes"], serde_json::json!(["shared"]));
        assert_eq!(v["confirmed"], false);
        assert_eq!(v["max_rate"], 768_000);
        assert_eq!(v["usb"]["vendor_id"], 0x2972);
        assert!(v.get("dsd").is_none() && v.get("note").is_none(), "{v}");
        // And it reads back
        let back: OutputDevice = serde_json::from_value(v).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn with_its_descriptors_it_also_tells_dsd_and_hardware_volume() {
        let raw = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../pm-usb-audio/tests/fixtures/fiio-ka17.bin"
        ))
        .unwrap();
        let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        let json = KA17.replace(
            r#""product":"FiiO KA17"}"#,
            &format!(r#""product":"FiiO KA17","descriptors":"{hex}"}}"#),
        );
        let d = describe(&parse_report(&json).unwrap()).unwrap();
        assert_eq!((d.dsd, d.hardware_volume), (Some(DsdKind::Native), true));
        assert_eq!(
            d.summary,
            "FiiO KA17 · PCM up to 768 kHz / 32 bit · DSD · hardware volume"
        );
        // Android listing no rates ("any rate"): the descriptors' clock can't be asked here, so none
        let any = json.replace(
            "[44100,48000,88200,96000,176400,192000,352800,384000,705600,768000]",
            "[]",
        );
        let d = describe(&parse_report(&any).unwrap()).unwrap();
        assert!(d.rates.is_empty() && d.max_rate == 0, "{:?}", d.rates);
        // Descriptors of another device (ids differ) are not used
        let other = json.replace("\"product_id\":147", "\"product_id\":148");
        let d = describe(&parse_report(&other).unwrap()).unwrap();
        assert_eq!((d.dsd, d.hardware_volume), (None, false));
        // Broken hex is ignored, not an error
        let broken = json.replace(&hex[..4], "zz");
        assert!(!describe(&parse_report(&broken).unwrap()).unwrap().hardware_volume);
    }

    #[test]
    fn names_people_know() {
        let dev = |name: &str, maker: Option<&str>, product: Option<&str>| ReportDevice {
            name: name.into(),
            kind: TYPE_USB_DEVICE,
            usb: Some(ReportUsb {
                manufacturer: maker.map(Into::into),
                product: product.map(Into::into),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            display_name(&dev("USB-Audio - KA17", Some("FiiO"), None)),
            "FiiO KA17"
        );
        assert_eq!(display_name(&dev("FiiO KA17", Some("FiiO"), None)), "FiiO KA17");
        assert_eq!(
            display_name(&dev(
                "",
                Some("Apple, Inc."),
                Some("USB-C to 3.5mm Headphone Jack Adapter")
            )),
            "Apple, Inc. USB-C to 3.5mm Headphone Jack Adapter"
        );
        assert_eq!(display_name(&dev("  KA17\u{0}\n ", None, None)), "KA17");
        assert_eq!(display_name(&dev("", None, None)), "USB audio device");
        assert_eq!(
            display_name(&dev(&"x".repeat(500), None, None)).chars().count(),
            MAX_NAME
        );
    }

    #[test]
    fn the_routed_usb_sink_wins_and_no_usb_sink_means_none() {
        let r = parse_report(
            r#"{"sdk":34,"devices":[
            {"id":1,"type":22,"name":"Headset","rates":[48000],"encodings":[2],"routed":false},
            {"id":2,"type":11,"name":"DAC","rates":[96000,44100,96000],"encodings":[21],"routed":true}]}"#,
        )
        .unwrap();
        let d = describe(&r).unwrap();
        assert_eq!(
            (d.name.as_str(), d.rates.as_slice(), d.bits.as_slice()),
            ("DAC", &[44_100, 96_000][..], &[24][..])
        );
        // Without routing information (Android 12): the first USB sink, a headset here
        let r =
            parse_report(r#"{"sdk":31,"devices":[{"type":22,"name":"Headset"},{"type":11,"name":"DAC"}]}"#)
                .unwrap();
        assert_eq!(describe(&r).unwrap().kind, DeviceKind::UsbHeadset);
        // The speaker, wired and Bluetooth headphones are not USB DACs
        let r =
            parse_report(r#"{"sdk":34,"devices":[{"type":2,"name":"Speaker"},{"type":8,"name":"Buds"}]}"#)
                .unwrap();
        assert_eq!(describe(&r), None);
        assert_eq!(
            describe(&parse_report(r#"{"v":1,"sdk":34,"devices":[]}"#).unwrap()),
            None
        );
    }

    #[test]
    fn hostile_reports_are_refused_or_cut_never_a_panic() {
        assert!(parse_report("").is_err());
        assert!(parse_report("not json").is_err());
        assert!(parse_report(r#"{"devices":[{"type":"eleven"}]}"#).is_err());
        assert!(parse_report(&" ".repeat(MAX_REPORT + 1)).is_err());
        // Unknown fields are fine, missing ones default
        let r = parse_report(r#"{"devices":[{"type":11,"future":true}],"later":1}"#).unwrap();
        assert_eq!(describe(&r).unwrap().name, "USB audio device");
        // Long lists are cut; absurd rates dropped
        let rates: Vec<String> = (0..1000).map(|i| (i * 1000).to_string()).collect();
        let json = format!(
            r#"{{"devices":[{{"type":11,"rates":[{},4000000000]}}]}}"#,
            rates.join(",")
        );
        let r = parse_report(&json).unwrap();
        assert_eq!(r.devices[0].rates.len(), MAX_ITEMS);
        let d = describe(&r).unwrap();
        assert!(d.rates.iter().all(|r| (8_000..=1_536_000).contains(r)));
        // A huge channel count saturates
        let r = parse_report(r#"{"devices":[{"type":11,"channel_counts":[4294967295]}]}"#).unwrap();
        assert_eq!(describe(&r).unwrap().channels, 255);
    }

    #[test]
    fn polls() {
        assert_eq!(parse_poll("7\n{\"v\":1}"), Some((7, "{\"v\":1}")));
        assert_eq!(parse_poll("7"), None);
        assert_eq!(parse_poll("x\n{}"), None);
        assert_eq!(parse_poll(""), None);
    }

    #[test]
    fn the_current_dac() {
        let d = describe(&parse_report(KA17).unwrap());
        assert!(set_current(d.clone()));
        assert!(!set_current(d.clone()), "the same DAC again is no change");
        assert_eq!(current(), d);
        assert!(set_current(None));
        assert_eq!(current(), None);
    }

    /// The Kotlin side (tools/release/android/UsbAudio.kt) writes every key read here, and nothing
    /// this side would drop: the report's contract, checked without a JVM.
    #[test]
    fn the_kotlin_side_writes_the_keys_read_here() {
        let kt = include_str!("../../../tools/release/android/UsbAudio.kt");
        let written: std::collections::BTreeSet<&str> = kt
            .match_indices(".put(\"")
            .filter_map(|(at, m)| kt[at + m.len()..].split('"').next())
            .collect();
        let read = [
            "v",
            "sdk",
            "devices",
            "id",
            "type",
            "name",
            "rates",
            "encodings",
            "channel_counts",
            "routed",
            "usb",
            "vendor_id",
            "product_id",
            "manufacturer",
            "product",
            "descriptors",
        ];
        assert_eq!(
            written,
            read.into_iter().collect(),
            "UsbAudio.kt writes {written:?}"
        );
        // And a report shaped like the one it builds parses
        let r = parse_report(
            r#"{"v":1,"sdk":35,"devices":[{"id":41,"type":11,"name":"FiiO KA17","rates":[],"encodings":[],
            "channel_counts":[],"routed":true,"usb":{"vendor_id":10610,"product_id":147,"manufacturer":"FiiO",
            "product":"FiiO KA17","descriptors":"12"}}]}"#,
        )
        .unwrap();
        assert_eq!(
            r.devices[0].usb.as_ref().unwrap().descriptors.as_deref(),
            Some("12")
        );
    }
}
