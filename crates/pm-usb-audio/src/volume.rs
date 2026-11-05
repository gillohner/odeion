//! The DAC's own volume (its UAC2 feature unit), and the never-louder hand-over when the exclusive
//! driver takes the DAC over from Android.
//!
//! While Android plays the DAC (shared mode) the listener's level is Android's media volume, which
//! Android applies to the samples. The exclusive driver sends the samples untouched, so the level
//! moves to the DAC's feature unit: before the first sample, the driver sets it to what Android's
//! media volume amounted to ([`handover_db`]), never more, so switching never gets louder.

/// A feature unit's volume range, in 1/256 dB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeRange {
    pub min: i16,
    pub max: i16,
    pub res: u16,
}

/// `db` as the feature unit's `CUR` value: within the range, on its resolution, rounded toward
/// quieter (never louder than asked).
pub fn db_to_cur(db: f32, range: VolumeRange) -> i16 {
    let want = (f64::from(db) * 256.0).floor();
    let clamped = want.clamp(f64::from(range.min), f64::from(range.max)) as i32;
    let res = i32::from(range.res.max(1));
    let steps = (clamped - i32::from(range.min)).div_euclid(res);
    (i32::from(range.min) + steps * res) as i16
}

/// A `CUR` value in dB.
pub fn cur_to_db(cur: i16) -> f32 {
    f32::from(cur) / 256.0
}

/// Android's default media volume curve (AOSP `DEFAULT_MEDIA_VOLUME_CURVE`: percent of the index
/// range to dB): 1 % at -58 dB, 20 % at -40, 60 % at -17, 100 % at 0.
const MEDIA_CURVE: [(f32, f32); 4] = [(1.0, -58.0), (20.0, -40.0), (60.0, -17.0), (100.0, 0.0)];
/// Below Android's own level by this much: phones tune their curves, and louder is never allowed.
pub const HANDOVER_MARGIN_DB: f32 = 3.0;

/// The level, in dB, that Android's media volume `percent` played at (its default curve), less
/// [`HANDOVER_MARGIN_DB`]; `None` for silence (0 %).
pub fn handover_db(percent: u8) -> Option<f32> {
    let p = f32::from(percent.min(100));
    if p <= 0.0 {
        return None;
    }
    let db = if p <= MEDIA_CURVE[0].0 {
        MEDIA_CURVE[0].1
    } else {
        MEDIA_CURVE
            .windows(2)
            .find(|w| p <= w[1].0)
            .map(|w| {
                let ((x0, y0), (x1, y1)) = (w[0], w[1]);
                y0 + (y1 - y0) * (p - x0) / (x1 - x0)
            })
            .unwrap_or(0.0)
    };
    Some(db - HANDOVER_MARGIN_DB)
}

/// The feature unit's starting `CUR` when the driver takes the DAC over: Android played at media
/// volume `percent` through the unit at `before` (its `CUR` then), so the level was both together;
/// never louder than that. Either unknown: the range's minimum.
pub fn handover_cur(percent: Option<u8>, before: Option<i16>, range: VolumeRange) -> i16 {
    match (percent.and_then(handover_db), before) {
        (Some(db), Some(b)) => db_to_cur(db + cur_to_db(b), range),
        _ => range.min,
    }
}

/// The unit's `CUR` when the driver gives the DAC back to Android: what it was before, or the
/// level the listener set since if that is quieter (Android's volume applies again on top).
pub fn giveback_cur(before: Option<i16>, now: i16) -> i16 {
    before.map_or(now, |b| b.min(now))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KA17: VolumeRange = VolumeRange {
        min: -127 * 256,
        max: 0,
        res: 128,
    };

    #[test]
    fn decibels_on_the_units_steps_never_louder() {
        assert_eq!(db_to_cur(0.0, KA17), 0);
        assert_eq!(db_to_cur(-10.0, KA17), -10 * 256);
        // -10.3 dB on 0.5 dB steps: -10.5, the quieter step
        assert_eq!(db_to_cur(-10.3, KA17), -10 * 256 - 128);
        assert_eq!(db_to_cur(6.0, KA17), 0, "never above the unit's maximum");
        assert_eq!(db_to_cur(-500.0, KA17), KA17.min);
        assert_eq!(cur_to_db(-10 * 256 - 128), -10.5);
    }

    /// The hand-over is never louder than what Android played at that media volume.
    #[test]
    fn the_hand_over_is_never_louder() {
        assert_eq!(handover_db(100), Some(-3.0));
        assert_eq!(handover_db(60), Some(-20.0));
        assert_eq!(handover_db(20), Some(-43.0));
        assert_eq!(handover_db(1), Some(-61.0));
        assert_eq!(handover_db(0), None);
        // Between points: on the line, less the margin
        assert_eq!(handover_db(40), Some(-31.5));
        // Every percent: the unit's step is at or below the hand-over level
        for p in 1..=100u8 {
            let mine = handover_db(p).unwrap();
            assert!(
                cur_to_db(handover_cur(Some(p), Some(0), KA17)) <= mine,
                "{p}%: the unit's step is not above it"
            );
        }
        // The unit was already turned down: the hand-over is that much lower
        assert_eq!(cur_to_db(handover_cur(Some(100), Some(-10 * 256), KA17)), -13.0);
        assert_eq!(
            handover_cur(None, Some(0), KA17),
            KA17.min,
            "unknown: the quietest"
        );
        assert_eq!(
            handover_cur(Some(50), None, KA17),
            KA17.min,
            "unknown: the quietest"
        );
        assert_eq!(handover_cur(Some(0), Some(0), KA17), KA17.min);
        // Back to Android: never above what it was before
        assert_eq!(giveback_cur(Some(-20 * 256), 0), -20 * 256);
        assert_eq!(giveback_cur(Some(-20 * 256), -30 * 256), -30 * 256);
        assert_eq!(giveback_cur(None, -5 * 256), -5 * 256);
        // A curve that only rises
        let mut last = f32::MIN;
        for p in 1..=100u8 {
            let d = handover_db(p).unwrap();
            assert!(d >= last, "{p}%");
            last = d;
        }
    }
}
