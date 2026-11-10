//! The phone's media volume (Android's `STREAM_MUSIC`) as the volume of the phone's own zone: one
//! volume for the app's slider, the phone's volume keys and Android's volume panel. Nothing is
//! scaled in the samples. Which volume it is follows the path ([`phone_kind`]):
//! * Exclusive USB on a DAC with a feature unit: the DAC's own unit, in dB (a remote session
//!   volume); Android's media volume isn't in the path and isn't touched;
//! * Android's bit-perfect path: Android's media volume (a local session), which Android hands to
//!   the DAC's own hardware volume (no gain in the samples); shared: the same media volume, applied
//!   by Android's mixer;
//! * a DAC without a volume of its own: none (a fixed session volume; set it on the DAC or the
//!   amplifier).
//!
//! The app's side (`MediaControl.kt`, over JNI) reads and sets the stream's index; this module
//! maps it to the zone's percent and back, so it is tested on every host:
//! * 100 % only at the top index;
//! * a percent lands on the index nearest to it, never on the top one below 100 %;
//! * a set to the percent already shown changes nothing (on a phone with more steps than 100,
//!   several indexes share a percent: the one Android has stays);
//! * up and down move one index: one key press.

use pm_model::{Volume, VolumeKind, VolumeLevel, ZoneVolume};
use std::sync::{Arc, RwLock};

/// `STREAM_MUSIC` as Android reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stream {
    pub index: u32,
    /// The top index (`getStreamMaxVolume`), at least 1.
    pub max: u32,
    pub muted: bool,
}

/// What Android does with the stream (the app's `MediaControl.kt`).
pub trait MediaVolumeBackend: Send + Sync {
    /// The stream now; `None` when Android can't say.
    fn stream(&self) -> Option<Stream>;
    fn set_index(&self, index: u32) -> Result<(), String>;
    fn set_muted(&self, muted: bool) -> Result<(), String>;
}

static BACKEND: RwLock<Option<Arc<dyn MediaVolumeBackend>>> = RwLock::new(None);

/// The app's side (set once at start).
pub fn set_backend(b: Arc<dyn MediaVolumeBackend>) {
    *BACKEND.write().unwrap_or_else(|e| e.into_inner()) = Some(b);
}

pub fn backend() -> Option<Arc<dyn MediaVolumeBackend>> {
    BACKEND.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The stream's index as the zone's percent: 100 only at the top index.
pub fn percent_of(index: u32, max: u32) -> u8 {
    let max = max.max(1);
    if index >= max {
        return 100;
    }
    let p = (u64::from(index) * 100 + u64::from(max) / 2) / u64::from(max);
    p.min(99) as u8
}

/// The index for `percent`, starting from `current`: `current` itself when it already shows that
/// percent, else the index nearest to it (the top one only for 100 %).
pub fn index_for(percent: u8, max: u32, current: u32) -> u32 {
    let max = max.max(1);
    let percent = percent.min(100);
    if percent_of(current.min(max), max) == percent {
        return current.min(max);
    }
    if percent == 100 {
        return max;
    }
    let i = ((u64::from(percent) * u64::from(max) + 50) / 100) as u32;
    i.min(max - 1)
}

/// What a zone volume command does to the stream: the new index, or a mute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Index(u32),
    Mute(bool),
}

/// `v` applied to the stream `s`; `None` for a level on another scale (dB: this volume is a
/// percent).
pub fn change_for(v: Volume, s: Stream) -> Option<Change> {
    let max = s.max.max(1);
    Some(match v {
        Volume::Percent(p) => Change::Index(index_for(p, max, s.index)),
        Volume::Up => Change::Index((s.index + 1).min(max)),
        Volume::Down => Change::Index(s.index.saturating_sub(1)),
        Volume::Mute(m) => Change::Mute(m),
        Volume::Db(_) => return None,
    })
}

/// The stream as the zone's level.
pub fn level_of(s: Stream) -> ZoneVolume {
    ZoneVolume {
        level: VolumeLevel::Percent(percent_of(s.index, s.max)),
        muted: s.muted,
    }
}

/// Android's media volume as the zone's level now (`None`: it can't be read here).
pub fn read() -> Option<ZoneVolume> {
    backend()?.stream().map(level_of)
}

/// Set Android's media volume from a zone volume command.
pub fn set(v: Volume) -> Result<(), String> {
    let b = backend().ok_or("this app can't set Android's media volume here")?;
    let s = b.stream().ok_or("Android's media volume could not be read")?;
    match change_for(v, s).ok_or("Android's media volume is a percent")? {
        Change::Index(i) if i == s.index => Ok(()),
        Change::Index(i) => b.set_index(i),
        Change::Mute(m) if m == s.muted => Ok(()),
        Change::Mute(m) => b.set_muted(m),
    }
}

/// Which volume the phone's output has now, by its live caps: the DAC's feature unit (dB) in
/// Exclusive USB, Android's media volume (percent), or none (a DAC without a volume of its own).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhoneVolume {
    Dac,
    Media,
    None,
}

pub fn phone_volume(hw: Option<VolumeKind>) -> PhoneVolume {
    match hw {
        Some(VolumeKind::Db { .. }) => PhoneVolume::Dac,
        Some(VolumeKind::Percent) => PhoneVolume::Media,
        Some(VolumeKind::Incremental) | None => PhoneVolume::None,
    }
}

/// Why the phone's output took no volume command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetError {
    /// It has no volume now (a DAC without a volume of its own: the DAC or amplifier sets it).
    NoVolume,
    Failed(String),
}

/// A zone volume command on the phone's output, whose live caps say `hw`.
pub fn phone_set(hw: Option<VolumeKind>, v: Volume) -> Result<(), SetError> {
    match phone_volume(hw) {
        PhoneVolume::Dac => crate::exclusive::set_volume(v).map_err(SetError::Failed),
        PhoneVolume::Media => {
            // An unmute also ends the mute kept for the DAC while the app doesn't hold it (the
            // next Exclusive USB take would bring it back otherwise)
            if v == Volume::Mute(false) {
                let _ = crate::exclusive::set_volume(v);
            }
            set(v).map_err(SetError::Failed)
        }
        PhoneVolume::None => Err(SetError::NoVolume),
    }
}

/// The phone output's volume now (its live caps): the DAC's unit (dB) in Exclusive USB, Android's
/// media volume (percent) in Android's bit-perfect or shared path, or none (a DAC without a volume
/// of its own).
pub fn phone_kind() -> Option<VolumeKind> {
    let shared = pm_model::OutputCaps {
        pcm_rates: vec![44_100, 48_000],
        max_pcm_bits: 24,
        pcm_bits: vec![16, 24],
        max_channels: 2,
        dsd: None,
        hw_volume: Some(VolumeKind::Percent),
        standby: false,
        source_switch: false,
        gapless: true,
        groupable: false,
        fixed_latency_ms: None,
        quality_ceiling: pm_model::QualityCeiling::LosslessUpTo {
            rate_hz: 48_000,
            bits: 24,
        },
    };
    crate::bit_perfect::phone_caps(&shared).hw_volume
}

/// The phone output's level now, whose live caps say `hw`.
pub fn phone_read(hw: Option<VolumeKind>) -> Option<ZoneVolume> {
    match phone_volume(hw) {
        PhoneVolume::Dac => crate::exclusive::volume(),
        PhoneVolume::Media => read(),
        PhoneVolume::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn full_only_at_the_top_index_and_every_percent_lands_nearest() {
        for max in 1..=250u32 {
            assert_eq!(percent_of(max, max), 100);
            assert_eq!(percent_of(0, max), 0);
            for i in 0..max {
                let p = percent_of(i, max);
                assert!(p < 100, "{i}/{max} shows 100 below the top");
                // From any index, the percent an index shows sets that index back (or one that
                // shows the same percent, on phones with more than 100 steps)
                for from in [0, i, max] {
                    let back = index_for(p, max, from);
                    assert_eq!(percent_of(back, max), p, "{i}/{max} from {from}");
                }
                // Already there: nothing moves
                assert_eq!(index_for(p, max, i), i);
            }
            for p in 0..100u8 {
                let i = index_for(p, max, max);
                assert!(i < max, "{p}% reaches the top of {max}");
                // Nearest: no other index is closer to the percent asked
                let d = |j: u32| (f64::from(j) * 100.0 / f64::from(max) - f64::from(p)).abs();
                assert!((0..max).all(|j| d(i) <= d(j) + 1e-9), "{p}% of {max}: {i}");
            }
            assert_eq!(index_for(100, max, 0), max);
        }
        // Monotonic: a higher percent is never a lower index
        for max in [15u32, 25, 30, 160] {
            let idx: Vec<u32> = (0..=100u8).map(|p| index_for(p, max, 0)).collect();
            assert!(idx.windows(2).all(|w| w[0] <= w[1]), "{max}: {idx:?}");
        }
    }

    #[test]
    fn a_key_press_is_one_index() {
        let s = Stream {
            index: 3,
            max: 15,
            muted: false,
        };
        assert_eq!(change_for(Volume::Up, s), Some(Change::Index(4)));
        assert_eq!(change_for(Volume::Down, s), Some(Change::Index(2)));
        let top = Stream { index: 15, ..s };
        assert_eq!(change_for(Volume::Up, top), Some(Change::Index(15)));
        let bottom = Stream { index: 0, ..s };
        assert_eq!(change_for(Volume::Down, bottom), Some(Change::Index(0)));
        assert_eq!(change_for(Volume::Mute(true), s), Some(Change::Mute(true)));
        assert_eq!(change_for(Volume::Db(-10.0), s), None, "a percent, not dB");
        assert_eq!(
            level_of(s),
            ZoneVolume {
                level: VolumeLevel::Percent(20),
                muted: false
            }
        );
    }

    /// Android as the app reports it: one stream, every call recorded.
    struct Fake {
        s: Mutex<Stream>,
        sets: Mutex<Vec<Change>>,
    }

    impl MediaVolumeBackend for Fake {
        fn stream(&self) -> Option<Stream> {
            Some(*self.s.lock().unwrap())
        }
        fn set_index(&self, index: u32) -> Result<(), String> {
            self.s.lock().unwrap().index = index;
            self.sets.lock().unwrap().push(Change::Index(index));
            Ok(())
        }
        fn set_muted(&self, muted: bool) -> Result<(), String> {
            self.s.lock().unwrap().muted = muted;
            self.sets.lock().unwrap().push(Change::Mute(muted));
            Ok(())
        }
    }

    #[test]
    fn the_phones_zone_volume_is_androids_media_volume() {
        let _one = crate::PHONE_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let fake = Arc::new(Fake {
            s: Mutex::new(Stream {
                index: 1,
                max: 25,
                muted: false,
            }),
            sets: Mutex::new(Vec::new()),
        });
        set_backend(fake.clone());
        let pct = Some(VolumeKind::Percent);
        assert_eq!(
            phone_read(pct),
            Some(ZoneVolume {
                level: VolumeLevel::Percent(4),
                muted: false
            })
        );
        // The slider at 4 % (where it already is): Android isn't touched
        phone_set(pct, Volume::Percent(4)).unwrap();
        assert!(fake.sets.lock().unwrap().is_empty());
        phone_set(pct, Volume::Percent(8)).unwrap();
        phone_set(pct, Volume::Up).unwrap();
        assert_eq!(*fake.sets.lock().unwrap(), [Change::Index(2), Change::Index(3)]);
        assert_eq!(
            phone_read(pct).unwrap().level,
            VolumeLevel::Percent(12),
            "read back as set"
        );
        phone_set(pct, Volume::Mute(true)).unwrap();
        assert!(phone_read(pct).unwrap().muted);
        // No volume of its own now (a DAC without one): nothing is set
        assert_eq!(phone_set(None, Volume::Percent(50)), Err(SetError::NoVolume));
        assert_eq!(phone_read(None), None);
        assert_eq!(fake.s.lock().unwrap().index, 3);
    }
}
