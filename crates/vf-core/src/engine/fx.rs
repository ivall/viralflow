//! Picture treatments the compositor applies to a layer beyond its colour
//! matrix: how the source is sampled (pixels, lens, mirror, colour fringes,
//! radial blur), what is added on top (a light leak) and which part of the box
//! stays visible (wipes, an iris, a clock hand). Effects and transitions both
//! produce them; `with` adds two together.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerFx {
    /// Red and blue shifted apart, as a share of the box width.
    pub chroma: f64,
    /// Block size in project pixels (0 = off).
    pub pixelate: f64,
    /// Colour levels per channel (0 = off).
    pub posterize: f64,
    /// Unsharp-mask strength, 0..2.
    pub sharpen: f64,
    /// Lens bulge (+) or pinch (−), −1..1.
    pub bulge: f64,
    /// The left half mirrored onto the right.
    pub mirror: bool,
    /// Radial blur towards the centre, 0..1.
    pub zoom_blur: f64,
    /// Warm light leak strength 0..1, and where it is (0..1 sweeps left to right).
    pub leak: f64,
    pub leak_phase: f64,
    /// Share of the box height hidden at the top and at the bottom.
    pub clip_top: f64,
    pub clip_bottom: f64,
    /// Radius of the visible circle as a share of the half diagonal (1 = all of the box).
    pub iris: f64,
    /// Visible share of a clockwise sweep from twelve o'clock (1 = all of the box).
    pub clock: f64,
}

impl Default for LayerFx {
    fn default() -> Self {
        LayerFx {
            chroma: 0.0,
            pixelate: 0.0,
            posterize: 0.0,
            sharpen: 0.0,
            bulge: 0.0,
            mirror: false,
            zoom_blur: 0.0,
            leak: 0.0,
            leak_phase: 0.0,
            clip_top: 0.0,
            clip_bottom: 0.0,
            iris: 1.0,
            clock: 1.0,
        }
    }
}

impl LayerFx {
    /// Both treatments at once: strengths add, the coarser sampling and the
    /// smaller visible part win.
    pub fn with(self, other: LayerFx) -> LayerFx {
        let posterize = match (self.posterize > 0.0, other.posterize > 0.0) {
            (true, true) => self.posterize.min(other.posterize),
            (true, false) => self.posterize,
            _ => other.posterize,
        };
        let (leak, leak_phase) = if other.leak > self.leak { (other.leak, other.leak_phase) } else { (self.leak, self.leak_phase) };
        LayerFx {
            chroma: self.chroma + other.chroma,
            pixelate: self.pixelate.max(other.pixelate),
            posterize,
            sharpen: (self.sharpen + other.sharpen).min(2.0),
            bulge: (self.bulge + other.bulge).clamp(-1.0, 1.0),
            mirror: self.mirror || other.mirror,
            zoom_blur: self.zoom_blur.max(other.zoom_blur).min(1.0),
            leak,
            leak_phase,
            clip_top: self.clip_top.max(other.clip_top),
            clip_bottom: self.clip_bottom.max(other.clip_bottom),
            iris: self.iris.min(other.iris),
            clock: self.clock.min(other.clock),
        }
    }

    pub fn is_idle(&self) -> bool {
        *self == LayerFx::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combining_keeps_the_stronger_and_the_smaller() {
        let a = LayerFx { chroma: 0.01, pixelate: 8.0, posterize: 6.0, iris: 0.5, clip_top: 0.1, ..Default::default() };
        let b = LayerFx { chroma: 0.02, pixelate: 20.0, posterize: 0.0, iris: 0.8, clip_bottom: 0.2, mirror: true, ..Default::default() };
        let both = a.with(b);
        assert!((both.chroma - 0.03).abs() < 1e-12);
        assert_eq!((both.pixelate, both.posterize, both.iris, both.clip_top, both.clip_bottom), (20.0, 6.0, 0.5, 0.1, 0.2));
        assert!(both.mirror);
        assert!(LayerFx::default().with(LayerFx::default()).is_idle());
    }
}
