//! Composable mission step documents; no application shell or default renderer.
pub use fractal_ui_model::missions::{Mission, StepState};

/// An embedding application executes step expansion itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// The stable `step:{mission}:{name}` expansion key.
    ToggleStep(String),
}

#[cfg(feature = "ratatui")]
mod render;
#[cfg(feature = "ratatui")]
pub use render::*;
