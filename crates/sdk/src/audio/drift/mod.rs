//! Drift compensation (ADR 0022): the controller decides the correction, the
//! adaptive resampler applies it.

pub(crate) mod controller;
pub(crate) mod resampler;
