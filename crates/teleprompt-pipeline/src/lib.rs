//! A script to its timeline and manifest (docs/design.md#crates).
//!
//! - [`schedule`]: policies, the scheduler, the [`Timeline`](schedule::Timeline)
//!   and its diff. Pure.
//! - [`compile`]: walks a resolved program, has the scene plugins validate
//!   and split action blocks, takes each line's duration from the voice
//!   cache or a take, and schedules the items. It plans: it never reaches
//!   a voice backend and never runs a scene's tool, so it uses only the
//!   scene crate's contract.

pub mod compile;
pub mod schedule;
