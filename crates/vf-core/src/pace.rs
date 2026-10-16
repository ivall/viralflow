//! Smooth progress and time left for jobs that report rarely and in jumps (Whisper
//! reports once per 20 s chunk of audio). Between reports the bar creeps toward
//! where the next report is expected and never passes it; the time left comes
//! from the measured rate once there is one. Times are in seconds.

#[derive(Debug, Clone)]
pub struct Pace {
    started: f64,
    floor: f64,
    reported_at: f64,
    step: f64,
    rate: f64,
    shown: f64,
}

impl Pace {
    /// `expected`: estimated length of the whole job; `first_step`: share of the job before the first report.
    pub fn new(expected: f64, first_step: f64, now: f64) -> Self {
        Pace { started: now, floor: 0.0, reported_at: now, step: first_step.clamp(0.01, 1.0), rate: 1.0 / expected.max(0.5), shown: 0.0 }
    }

    pub fn report(&mut self, fraction: f64, now: f64) {
        let value = fraction.clamp(0.0, 1.0);
        if value <= self.floor {
            return;
        }
        // The last jump predicts the next one.
        self.step = (value - self.floor).max(0.01);
        self.floor = value;
        self.reported_at = now;
        let elapsed = now - self.started;
        if elapsed > 0.2 {
            self.rate = value / elapsed;
        }
    }

    /// Fraction to draw (0..1); never goes backwards.
    pub fn value(&mut self, now: f64) -> f64 {
        let ceiling = (self.floor + self.step).min(1.0);
        let expected_wait = self.step / self.rate;
        let waited = (now - self.reported_at).max(0.0) / expected_wait.max(0.05);
        // Stops short of the ceiling: only a report may move the bar there.
        let creep = (ceiling - self.floor) * 0.95 * (1.0 - (-1.5 * waited).exp());
        self.shown = self.shown.max((self.floor + creep).min(0.995));
        self.shown
    }

    pub fn remaining(&mut self, now: f64) -> f64 {
        ((1.0 - self.value(now)) / self.rate).max(0.0)
    }

    pub fn elapsed(&self, now: f64) -> f64 {
        (now - self.started).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creeps_without_passing_the_next_report() {
        let mut pace = Pace::new(100.0, 0.2, 0.0);
        let early = pace.value(5.0);
        let late = pace.value(500.0);
        assert!(early > 0.0 && late > early && late < 0.2);
    }

    #[test]
    fn uses_the_measured_rate() {
        let mut pace = Pace::new(100.0, 0.5, 0.0);
        let before = pace.value(30.0);
        pace.report(0.1, 30.0);
        assert!(pace.value(30.0) >= before);
        assert!(pace.remaining(30.0) > 200.0);
    }
}
