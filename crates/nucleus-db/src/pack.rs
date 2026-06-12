//! CMSIS Device Family Pack parser (placeholder).
//!
//! Phase 1 will parse the GPIO alternate-function tables from a CMSIS Device
//! Family Pack (the `.pdsc` + GPIO XML) into the normalized [`crate::AfMapping`]
//! model, replacing the hand-verified seed in [`crate::data`]. The vendored
//! `cmsis-device-f4` register headers do **not** carry this mux data, so the
//! AF-table source must be added before this is implemented.
//!
//! Tracked as remaining Phase 1 work; intentionally unimplemented for now.
