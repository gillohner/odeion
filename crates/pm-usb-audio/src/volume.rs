//! The DAC's own volume (its UAC2 feature unit), and the never-louder hand-over when the exclusive
//! driver takes the DAC over from Android.
//!
//! While Android plays the DAC (shared mode) the listener's level is Android's media volume, which
//! Android applies to the samples, through the unit at whatever level (and mute) it had. The
//! exclusive driver sends the samples untouched, so the level moves to the DAC's feature unit:
//! before the first sample, the driver sets it to what both together amounted to ([`handover`]),
//! never more; when that is below what the unit can do, or unknown, it mutes. Giving the DAC back,
//! the unit is never left louder than it was ([`giveback`]).

/// One sub-range of a feature unit's volume (`RANGE` reply), in 1/256 dB: `min`, `min + res`, …
/// up to `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubRange {
    pub min: i16,
    pub max: i16,
    pub res: u16,
}

/// A feature unit's volume range: the sub-ranges it advertises, nothing in between.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeRange {
    pub subranges: Vec<SubRange>,
}

impl VolumeRange {
    /// One sub-range.
    pub fn single(min: i16, max: i16, res: u16) -> Self {
        VolumeRange {
            subranges: vec![SubRange { min, max, res }],
        }
    }

    /// The quietest value it takes.
    pub fn min(&self) -> i16 {
        self.subranges.iter().map(|r| r.min).min().unwrap_or(0)
    }

    /// The loudest value it takes.
    pub fn max(&self) -> i16 {
        self.subranges.iter().map(|r| r.max).max().unwrap_or(0)
    }

    /// The finest step, in dB (for a UI's slider).
    pub fn step_db(&self) -> f32 {
        let res = self
            .subranges
            .iter()
            .filter(|r| r.max > r.min)
            .map(|r| r.res)
            .min()
            .unwrap_or(256);
        f32::from(res.max(1)) / 256.0
    }

    /// The loudest value it takes at or below `cur` (1/256 dB); `None` when `cur` is below its
    /// minimum.
    pub fn at_or_below(&self, cur: i32) -> Option<i16> {
        self.subranges
            .iter()
            .filter_map(|r| {
                let (min, max) = (i32::from(r.min), i32::from(r.max));
                if cur < min {
                    return None;
                }
                if min == max {
                    return Some(r.min);
                }
                // On its grid: min, min + res, ... (max itself only when on the grid)
                let res = i32::from(r.res.max(1));
                let top = cur.min(max);
                Some((min + (top - min).div_euclid(res) * res) as i16)
            })
            .max()
    }
}

/// A feature unit's level: `CUR` (1/256 dB) and mute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub cur: i16,
    pub mute: bool,
}

/// `db` as a `CUR` value the unit takes, at or below it; the minimum when it is below that (the
/// listener's own choice of level, not a hand-over).
pub fn db_to_cur(db: f32, range: &VolumeRange) -> i16 {
    let want = (f64::from(db) * 256.0).floor() as i32;
    range.at_or_below(want).unwrap_or_else(|| range.min())
}

/// A `CUR` value in dB.
pub fn cur_to_db(cur: i16) -> f32 {
    f32::from(cur) / 256.0
}

/// The unit's level when the driver takes the DAC over: Android played the DAC at `media_db` (its
/// own attenuation for the DAC's route, `AudioManager.getStreamVolumeDb`, read before the take
/// while Android still played the DAC) through the unit at `before`; never louder than both
/// together. Muted, at its minimum, when either is unknown, Android was silent, the unit was
/// muted, or the target is below what the unit can do: the listener raises it.
pub fn handover(media_db: Option<f32>, before: Option<Level>, range: &VolumeRange) -> Level {
    let quiet = Level {
        cur: range.min(),
        mute: true,
    };
    let (Some(db), Some(b)) = (media_db.filter(|d| d.is_finite()), before) else {
        return quiet;
    };
    let target = ((db + cur_to_db(b.cur)) * 256.0).floor() as i32;
    match range.at_or_below(target) {
        Some(cur) => Level { cur, mute: b.mute },
        None => quiet,
    }
}

/// The unit's level when the driver gives the DAC back to Android: exactly its own level and mute
/// from before the take (Android's volume applies on top again, as it did); unknown: as now.
pub fn giveback(before: Option<Level>, now: Level) -> Level {
    before.unwrap_or(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ka17() -> VolumeRange {
        VolumeRange::single(-127 * 256, 0, 128)
    }

    fn at(cur: i16) -> Option<Level> {
        Some(Level { cur, mute: false })
    }

    #[test]
    fn decibels_on_the_units_steps_never_louder() {
        let r = ka17();
        assert_eq!(db_to_cur(0.0, &r), 0);
        assert_eq!(db_to_cur(-10.0, &r), -10 * 256);
        // -10.3 dB on 0.5 dB steps: -10.5, the quieter step
        assert_eq!(db_to_cur(-10.3, &r), -10 * 256 - 128);
        assert_eq!(db_to_cur(6.0, &r), 0, "never above the unit's maximum");
        assert_eq!(db_to_cur(-500.0, &r), r.min());
        assert_eq!(cur_to_db(-10 * 256 - 128), -10.5);
        assert_eq!(r.step_db(), 0.5);
    }

    /// Codex M (round 1 of #306): several sub-ranges are kept apart; nothing between them, or off
    /// their steps, is ever chosen; a zero resolution is a single value.
    #[test]
    fn only_values_the_unit_advertises() {
        let r = VolumeRange {
            subranges: vec![
                SubRange {
                    min: -60 * 256,
                    max: -40 * 256,
                    res: 512,
                },
                SubRange {
                    min: -20 * 256,
                    max: 0,
                    res: 256,
                },
                SubRange {
                    min: -30 * 256,
                    max: -30 * 256,
                    res: 0,
                },
            ],
        };
        assert_eq!(
            r.at_or_below(-25 * 256),
            Some(-30 * 256),
            "the gap: the value below it"
        );
        assert_eq!(r.at_or_below(-35 * 256), Some(-40 * 256));
        assert_eq!(r.at_or_below(-41 * 256), Some(-42 * 256), "2 dB steps from -60");
        assert_eq!(r.at_or_below(-10 * 256 - 3), Some(-11 * 256));
        assert_eq!(r.at_or_below(-61 * 256), None, "below the minimum");
        assert_eq!((r.min(), r.max()), (-60 * 256, 0));
    }

    /// The hand-over is Android's measured attenuation for the DAC plus the unit's level then, on
    /// the unit's steps, rounded toward quieter: never louder.
    #[test]
    fn the_hand_over_is_never_louder() {
        let r = ka17();
        for tenth in 0..=600 {
            let db = -(tenth as f32) / 10.0;
            let l = handover(Some(db), at(0), &r);
            assert!(!l.mute && cur_to_db(l.cur) <= db, "{db} dB");
        }
        assert_eq!(
            handover(Some(-20.0), at(0), &r),
            Level {
                cur: -20 * 256,
                mute: false
            }
        );
        // The unit was already turned down: the hand-over is that much lower
        assert_eq!(
            handover(Some(0.0), at(-10 * 256), &r),
            Level {
                cur: -10 * 256,
                mute: false
            }
        );
        assert_eq!(
            handover(Some(-3.3), at(-10 * 256), &r),
            Level {
                cur: -13 * 256 - 128,
                mute: false
            }
        );
    }

    /// Codex H3 (round 2 of #306): no assumed curve: without Android's own reading for the DAC
    /// (an old Android, or not read from the DAC's route) the DAC starts muted at its minimum.
    #[test]
    fn no_reading_no_sound() {
        let r = ka17();
        let quiet = Level {
            cur: r.min(),
            mute: true,
        };
        assert_eq!(handover(None, at(0), &r), quiet);
        assert_eq!(
            handover(Some(f32::NEG_INFINITY), at(0), &r),
            quiet,
            "Android silent"
        );
        assert_eq!(handover(Some(f32::NAN), at(0), &r), quiet);
    }

    /// Codex H2 / Claude M3 (round 1 of #306): below what the unit can do, or muted before: muted;
    /// Claude M4 (round 2): the give-back is the unit's own level and mute from before the take.
    #[test]
    fn what_the_unit_cant_do_quietly_is_muted() {
        let shallow = VolumeRange::single(-20 * 256, 0, 256);
        let quiet = Level {
            cur: -20 * 256,
            mute: true,
        };
        assert_eq!(
            handover(Some(-43.0), at(0), &shallow),
            quiet,
            "-43 dB is below -20 dB: muted"
        );
        assert_eq!(
            handover(Some(0.0), None, &shallow),
            quiet,
            "the unit's level unknown"
        );
        assert_eq!(
            handover(
                Some(-3.0),
                Some(Level {
                    cur: -6 * 256,
                    mute: true
                }),
                &shallow
            ),
            Level {
                cur: -9 * 256,
                mute: true
            },
            "muted before: muted"
        );
        // Back to Android: exactly as before, whatever the hand-over or the listener did
        let before = Some(Level {
            cur: -20 * 256,
            mute: false,
        });
        assert_eq!(
            giveback(before, Level { cur: 0, mute: false }),
            Level {
                cur: -20 * 256,
                mute: false
            }
        );
        assert_eq!(
            giveback(
                before,
                Level {
                    cur: -127 * 256,
                    mute: true
                }
            ),
            Level {
                cur: -20 * 256,
                mute: false
            }
        );
        assert_eq!(
            giveback(Some(Level { cur: 0, mute: true }), Level { cur: 0, mute: false }),
            Level { cur: 0, mute: true }
        );
        assert_eq!(
            giveback(
                None,
                Level {
                    cur: -5 * 256,
                    mute: false
                }
            ),
            Level {
                cur: -5 * 256,
                mute: false
            }
        );
    }
}
