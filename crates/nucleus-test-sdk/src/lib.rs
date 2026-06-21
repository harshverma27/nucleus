//! Nucleus M7 host SDK (`nucleus_test_sdk`): the power-user escape hatch for
//! scripted, host-driven HIL tests. Drives the on-device test-agent over a
//! fixed-address RAM mailbox (via any [`nucleus_hil::backend::Backend`]'s
//! `read_mem32`/`write_mem32`) and the device UART over the ST-Link VCP.

pub mod agent;
pub mod protocol;
pub mod serial;

pub use agent::{AgentClient, SdkError};
pub use serial::Serial;
