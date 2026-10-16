//! Exact time. Media timestamps are integers in a stream's time base (a
//! fraction such as 1/15360 s), and the timeline moves in whole frames of the
//! project's rate; going through these instead of adding seconds as floats
//! keeps "the frame at 4.4 s" from becoming the frame before it because
//! 4.8 - 0.4 is 4.3999999999999995. The document still stores seconds (as the
//! file format always has); they are snapped to frames where it matters.

/// A fraction `num / den` with `den > 0`, kept reduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.abs().max(1)
}

impl Rational {
    pub fn new(num: i64, den: i64) -> Self {
        let (num, den) = if den < 0 { (-num, -den) } else { (num, den.max(1)) };
        let divisor = gcd(num, den);
        Rational { num: num / divisor, den: den / divisor }
    }

    /// One frame at `fps` frames per second.
    pub fn frame(fps: u32) -> Self {
        Rational::new(1, fps.max(1) as i64)
    }

    pub fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Seconds of `ticks` units of this fraction (exact up to the final division).
    pub fn seconds(self, ticks: i64) -> f64 {
        (ticks as i128 * self.num as i128) as f64 / self.den as f64
    }

    /// The tick whose span holds `seconds` (with a tolerance of a millionth of
    /// a tick for values that went through float arithmetic).
    pub fn tick_at(self, seconds: f64) -> i64 {
        let ticks = seconds * self.den as f64 / self.num.max(1) as f64;
        (ticks + 1e-6).floor() as i64
    }

    /// The nearest tick to `seconds`.
    pub fn nearest_tick(self, seconds: f64) -> i64 {
        (seconds * self.den as f64 / self.num.max(1) as f64).round() as i64
    }
}

/// Index of the frame nearest to `time` at `fps`.
pub fn frame_index(time: f64, fps: u32) -> i64 {
    Rational::frame(fps).nearest_tick(time)
}

/// Start of frame `index` at `fps`.
pub fn frame_time(index: i64, fps: u32) -> f64 {
    Rational::frame(fps).seconds(index)
}

/// `time` moved onto the nearest frame boundary.
pub fn snap_to_frame(time: f64, fps: u32) -> f64 {
    frame_time(frame_index(time, fps), fps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_reduce_and_convert() {
        assert_eq!(Rational::new(512, 15360), Rational::new(1, 30));
        assert_eq!(Rational::new(3, -6), Rational { num: -1, den: 2 });
        let base = Rational::new(1, 15360);
        assert_eq!(base.seconds(67584), 4.4);
        assert_eq!(base.tick_at(4.3999999999999995), 67584);
        assert_eq!(Rational::new(1001, 30000).seconds(30000), 1001.0);
    }

    #[test]
    fn frames_survive_float_noise() {
        assert_eq!(frame_index(4.8 - 0.4, 30), 132);
        assert_eq!(frame_time(132, 30), 4.4);
        assert_eq!(snap_to_frame(0.1 + 0.2, 30), 0.3);
        assert_eq!(snap_to_frame(1.0 / 3.0, 24), 8.0 / 24.0);
    }
}
