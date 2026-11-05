//! How many frames go in each isochronous packet, and what an asynchronous DAC's feedback says.
//!
//! A packet goes out once per service interval: every `2^(bInterval-1)` frames of 1 ms at full
//! speed, or microframes of 125 µs at high speed. At 44.1 kHz on full speed that is 44.1 frames a
//! packet: nine of 44 and one of 45 in every ten. An asynchronous DAC keeps its own clock and
//! reports, on its feedback endpoint, how many frames it takes per (micro)frame; the schedule then
//! follows that instead of the nominal rate, so its buffer neither runs dry nor overflows.

/// The bus speed the DAC runs at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speed {
    /// 12 Mbit/s: 1 ms frames.
    Full,
    /// 480 Mbit/s: 125 µs microframes.
    High,
}

/// (Micro)frames per second.
fn frames_per_second(speed: Speed) -> u32 {
    match speed {
        Speed::Full => 1000,
        Speed::High => 8000,
    }
}

/// (Micro)frames between packets for an endpoint's `bInterval`.
pub fn period(interval: u8) -> u32 {
    1 << interval.clamp(1, 16).saturating_sub(1)
}

/// Packets per second.
pub fn packets_per_second(speed: Speed, interval: u8) -> u32 {
    frames_per_second(speed) / period(interval)
}

/// Frames per packet at `rate`, in 16.16 fixed point.
pub fn nominal_q16(rate: u32, speed: Speed, interval: u8) -> u64 {
    (u64::from(rate) << 16) / u64::from(packets_per_second(speed, interval))
}

/// The packet sizes, in frames, one after another: frames per packet as an exact fraction
/// (`num / den`), the remainder carried, so no frame is lost or gained over time (44.1 kHz at full
/// speed: exactly 44 100 frames in 1000 packets).
#[derive(Debug, Clone)]
pub struct Schedule {
    num: u64,
    den: u64,
    /// The nominal rate per packet, 16.16 (the feedback's sanity window).
    nominal: u64,
    /// The carried remainder, in 1/den frames.
    acc: u64,
    period: u32,
}

impl Schedule {
    pub fn new(rate: u32, speed: Speed, interval: u8) -> Self {
        Self {
            num: u64::from(rate),
            den: u64::from(packets_per_second(speed, interval).max(1)),
            nominal: nominal_q16(rate, speed, interval),
            acc: 0,
            period: period(interval),
        }
    }

    /// The next packet's frames.
    pub fn next_packet(&mut self) -> u32 {
        self.acc += self.num;
        let n = self.acc / self.den;
        self.acc %= self.den;
        n as u32
    }

    /// The next `n` packets' frames.
    pub fn packets(&mut self, n: usize) -> Vec<u32> {
        (0..n).map(|_| self.next_packet()).collect()
    }

    /// Follow the DAC's feedback (frames per (micro)frame, 16.16): per packet, that times the
    /// period. A value off the nominal rate by more than an eighth is ignored (a misread).
    pub fn follow(&mut self, frames_per_frame_q16: u64) -> bool {
        let per_packet = frames_per_frame_q16 * u64::from(self.period);
        let lo = self.nominal - self.nominal / 8;
        let hi = self.nominal + self.nominal / 8;
        if !(lo..=hi).contains(&per_packet) {
            return false;
        }
        // The carried part, in the new unit
        self.acc = self.acc * 65_536 / self.den;
        self.num = per_packet;
        self.den = 65_536;
        true
    }

    /// The most frames a packet may carry now (rounded up, one more for the carry).
    pub fn max_packet_frames(&self) -> u32 {
        self.num.div_ceil(self.den) as u32 + 1
    }
}

/// A feedback reading as frames per (micro)frame, 16.16: full speed sends 10.14 in 3 bytes, high
/// speed 16.16 in 4. Some full-speed DACs send 4 bytes of 16.16 (`four_byte_fs`, a quirk).
pub fn decode_feedback(reply: &[u8], speed: Speed, four_byte_fs: bool) -> Option<u64> {
    match (speed, reply.len(), four_byte_fs) {
        (Speed::Full, 3.., false) => {
            Some(u64::from(u32::from_le_bytes([reply[0], reply[1], reply[2], 0])) << 2)
        }
        (Speed::Full, 4.., true) | (Speed::High, 4.., _) => Some(u64::from(u32::from_le_bytes([
            reply[0], reply[1], reply[2], reply[3],
        ]))),
        _ => None,
    }
    .filter(|v| *v > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sum(s: &mut Schedule, n: usize) -> u64 {
        s.packets(n).iter().map(|f| u64::from(*f)).sum()
    }

    #[test]
    fn packet_sizes_add_up_to_the_rate() {
        // Full speed, every frame: 44.1 kHz is nine packets of 44 and one of 45
        let mut s = Schedule::new(44_100, Speed::Full, 1);
        let ten = s.packets(10);
        assert_eq!(ten.iter().filter(|f| **f == 44).count(), 9, "{ten:?}");
        assert_eq!(ten.iter().filter(|f| **f == 45).count(), 1, "{ten:?}");
        assert_eq!(sum(&mut Schedule::new(44_100, Speed::Full, 1), 1000), 44_100);
        assert!(Schedule::new(48_000, Speed::Full, 1)
            .packets(100)
            .iter()
            .all(|f| *f == 48));
        // High speed, every microframe (the KA17): 96 kHz is 12 a packet, 192 kHz 24
        assert!(Schedule::new(96_000, Speed::High, 1)
            .packets(8000)
            .iter()
            .all(|f| *f == 12));
        assert!(Schedule::new(192_000, Speed::High, 1)
            .packets(8000)
            .iter()
            .all(|f| *f == 24));
        // 44.1 kHz at high speed: 5.5125 a microframe, exactly 44100 a second
        let mut hs = Schedule::new(44_100, Speed::High, 1);
        let one = hs.packets(8000);
        assert!(one.iter().all(|f| *f == 5 || *f == 6));
        assert_eq!(one.iter().map(|f| u64::from(*f)).sum::<u64>(), 44_100);
        // High speed every 8 microframes (bInterval 4): one packet a millisecond
        assert_eq!(packets_per_second(Speed::High, 4), 1000);
        assert!(Schedule::new(48_000, Speed::High, 4)
            .packets(10)
            .iter()
            .all(|f| *f == 48));
        assert_eq!(packets_per_second(Speed::Full, 1), 1000);
        assert_eq!(Schedule::new(96_000, Speed::High, 1).max_packet_frames(), 13);
    }

    #[test]
    fn feedback_moves_the_packet_sizes() {
        // The DAC runs a little fast: 12.1 frames a microframe at 96 kHz
        let mut s = Schedule::new(96_000, Speed::High, 1);
        let fb = (12u64 << 16) + (1u64 << 16) / 10;
        let bytes = (fb as u32).to_le_bytes();
        assert_eq!(decode_feedback(&bytes, Speed::High, false), Some(fb));
        assert!(s.follow(decode_feedback(&bytes, Speed::High, false).unwrap()));
        // 12.1 less the 16.16 rounding: 1209 or 1210 frames in 100 packets
        let hundred: u64 = s.packets(100).iter().map(|f| u64::from(*f)).sum();
        assert!((1_209..=1_210).contains(&hundred), "{hundred}");
        // Full speed 10.14: 44.1 frames a millisecond
        let fs = ((44.1f64 * 16384.0).round() as u32).to_le_bytes();
        let v = decode_feedback(&fs[..3], Speed::Full, false).unwrap();
        assert_eq!(v >> 16, 44);
        let mut f = Schedule::new(44_100, Speed::Full, 1);
        assert!(f.follow(v));
        // 10.14 holds 44.1 to within 1/16384 of a frame: 44 099 or 44 100 in a second
        assert!((44_099..=44_100).contains(&sum(&mut f, 1000)));
        // A misread far off the rate is ignored; short replies give nothing
        let mut s = Schedule::new(96_000, Speed::High, 1);
        assert!(!s.follow(24u64 << 16));
        assert!(s.packets(8).iter().all(|f| *f == 12));
        assert_eq!(decode_feedback(&[1, 2], Speed::High, false), None);
        assert_eq!(decode_feedback(&[0, 0, 0, 0], Speed::High, false), None);
        // The quirk: four bytes of 16.16 at full speed
        let q = ((48u32) << 16).to_le_bytes();
        assert_eq!(decode_feedback(&q, Speed::Full, true), Some(48u64 << 16));
    }
}
