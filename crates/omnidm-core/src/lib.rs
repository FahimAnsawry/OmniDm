// SPDX-License-Identifier: GPL-3.0-or-later

//! Download engine: segments, splitting and ramp-up, resume checks, speed limiting.

pub mod engine;
pub mod meter;
pub mod ramp;
pub mod segment;

pub use ramp::{RampController, RampInput, RampMode, RampPolicy};
pub use segment::{MapError, Segment, SegmentId, SegmentMap, SplitPlan, SplitPolicy};
