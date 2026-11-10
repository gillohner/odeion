//! The DAC's own volume (its UAC2 feature unit), the start level when the exclusive driver takes
//! the DAC over from Android (never louder than the DAC was), and the never-louder give-back.
//!
//! The exclusive driver sends the samples untouched, so the level is the DAC's feature unit. Before
//! the first sample the driver sets it to a known start level ([`start_level`]): the level the app
//! last played this DAC at, else a low [`START_DB`], on the unit's steps and within its range.
//! Android's media volume plays no part in it (while the app holds the DAC, Android routes media
//! to the phone's speaker). Giving the DAC back, the unit is never left louder than it was
//! ([`giveback`]).

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
/// listener's own choice of level, or a start level).
pub fn db_to_cur(db: f32, range: &VolumeRange) -> i16 {
    let want = (f64::from(db) * 256.0).floor() as i32;
    range.at_or_below(want).unwrap_or_else(|| range.min())
}

/// A `CUR` value in dB.
pub fn cur_to_db(cur: i16) -> f32 {
    f32::from(cur) / 256.0
}

/// The start level of a DAC the app has not played before: low, so the first sound is quiet.
pub const START_DB: f32 = -40.0;

/// The unit's level before the first sample once the driver took the DAC over: `saved_db` (the
/// level the app last played this DAC at), else [`START_DB`]; on the unit's steps, rounded toward
/// quieter, at most its maximum and, when the unit can't go that low, its minimum. Never muted:
/// what the listener hears first is a known, low level.
pub fn start_level(saved_db: Option<f32>, range: &VolumeRange) -> Level {
    let db = saved_db.filter(|d| d.is_finite()).unwrap_or(START_DB);
    Level {
        cur: db_to_cur(db, range),
        mute: false,
    }
}

/// [`start_level`], never louder than the unit was before the take (`before`, when known): the
/// lower of the two; a unit muted before starts at the lower of the start level and its minimum,
/// unmuted, so it is heard, quietly. A DAC turned down (or muted) on its own buttons stays down.
pub fn start_level_below(saved_db: Option<f32>, before: Option<Level>, range: &VolumeRange) -> Level {
    let start = start_level(saved_db, range);
    let cur = match before {
        Some(b) if b.mute => start.cur.min(range.min()),
        Some(b) => start.cur.min(b.cur),
        None => start.cur,
    };
    Level { cur, mute: false }
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

    /// The start level: the level the app last played this DAC at, else -40 dB; on the unit's
    /// steps toward quieter, within its range; never derived from Android's media volume (it
    /// isn't an input), never muted.
    #[test]
    fn the_start_level_is_the_last_one_else_minus_40_db() {
        let r = ka17();
        assert_eq!(
            start_level(None, &r),
            Level {
                cur: -40 * 256,
                mute: false
            }
        );
        assert_eq!(
            start_level(Some(-23.0), &r),
            Level {
                cur: -23 * 256,
                mute: false
            }
        );
        // -23.3 dB on 0.5 dB steps: -23.5, the quieter step
        assert_eq!(start_level(Some(-23.3), &r).cur, -23 * 256 - 128);
        // Never above the unit's maximum; a broken saved value is no saved value
        assert_eq!(start_level(Some(6.0), &r).cur, 0);
        assert_eq!(start_level(Some(f32::NAN), &r).cur, -40 * 256);
        assert_eq!(start_level(Some(f32::INFINITY), &r).cur, -40 * 256);
        // A unit that can't go as low as -40 dB: its minimum, still not muted
        let shallow = VolumeRange::single(-20 * 256, 0, 256);
        assert_eq!(
            start_level(None, &shallow),
            Level {
                cur: -20 * 256,
                mute: false
            }
        );
        assert_eq!(start_level(Some(-90.0), &shallow).cur, -20 * 256);
    }

    /// Never louder than the DAC was before the take: the lower of the start level and the level
    /// before; muted before: its minimum, heard; unknown before: the start level.
    #[test]
    fn the_start_level_is_never_louder_than_before() {
        let r = ka17();
        let at = |db: i16, mute: bool| Some(Level { cur: db * 256, mute });
        let heard = |db: i16| Level {
            cur: db * 256,
            mute: false,
        };
        // Turned down to -60 dB on its own buttons, -5 dB kept from last time: -60 dB
        assert_eq!(start_level_below(Some(-5.0), at(-60, false), &r), heard(-60));
        // At 0 dB before: the start level (-40, or the one kept)
        assert_eq!(start_level_below(None, at(0, false), &r), heard(-40));
        assert_eq!(start_level_below(Some(-23.0), at(0, false), &r), heard(-23));
        // Muted before: its minimum, unmuted
        assert_eq!(start_level_below(Some(-5.0), at(0, true), &r), heard(-127));
        // Not known before: the start level
        assert_eq!(start_level_below(None, None, &r), heard(-40));
    }

    /// Claude M4 (round 2 of #306): the give-back is the unit's own level and mute from before
    /// the take.
    #[test]
    fn the_give_back_is_the_level_from_before() {
        // Back to Android: exactly as before, whatever the start level or the listener did
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
