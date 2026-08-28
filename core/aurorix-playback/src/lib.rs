//! Platform-neutral playback session, command, and presentation-clock contracts.

pub mod clock;
pub mod command;
pub mod failure;
pub mod pipeline;
pub mod playback_facts;
pub mod queue;
pub mod queue_policy;
pub mod session;
pub mod source_resolution;
pub mod transition;

#[cfg(test)]
mod contract_tests;
