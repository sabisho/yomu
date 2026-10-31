//! Application view presentation adapters.
//!
//! Bridges the `Yomu` application entity, background decode workers, selection
//! state, and action dispatchers with the presentation components in `src/ui/`.

pub mod collections;
pub mod continue_reading;
pub mod details;
pub mod history;
pub mod library;
pub mod overlays;
pub mod settings;
pub mod shell;
