//! Use these policies for all future screen motion, and Base's transition
//! engine (which reads App::reduce_motion) for interpolation.
use std::time::Duration;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Feedback,
    Toggle,
    Popup,
    SearchPulse,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    pub duration: Duration,
    pub scale: f32,
    pub translate: f32,
}
impl Motion {
    pub fn resolve(self, reduced: bool) -> Transition {
        if reduced {
            return Transition {
                duration: Duration::ZERO,
                scale: 1.,
                translate: 0.,
            };
        }
        let (ms, scale, translate) = match self {
            Self::Feedback => (120, 0.97, 0.),
            Self::Toggle => (150, 1., 12.),
            Self::Popup => (120, 0.98, 0.),
            Self::SearchPulse => (650, 1., 0.),
        };
        Transition {
            duration: Duration::from_millis(ms),
            scale,
            translate,
        }
    }
}
