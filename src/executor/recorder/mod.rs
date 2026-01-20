//! Test recorder module - record player actions to create tests

mod actions;
mod bounding_box;
mod state;
#[cfg(test)]
mod tests;

pub use state::RecorderState;

