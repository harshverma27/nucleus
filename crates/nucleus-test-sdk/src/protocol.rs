//! Wire constants for the device test-agent RAM mailbox (protocol v1).
//! Mirrored byte-for-byte by the agent firmware in
//! `nucleus-hil/tests/fixtures/agent_loopback/agent.c`.

/// Mailbox base address: start of SRAM, pinned by the agent's linker script
/// (`.nucleus_agent` section). The host needs no ELF symbol lookup.
pub const MAILBOX_BASE: u32 = 0x2000_0000;

/// `'NTAg'` little-endian — agent writes this once initialized.
pub const MAGIC: u32 = 0x4E54_4167;
pub const PROTO_VERSION: u32 = 1;

// Field offsets from MAILBOX_BASE.
pub const OFF_MAGIC: u32 = 0x00;
pub const OFF_VERSION: u32 = 0x04;
pub const OFF_SEQ: u32 = 0x08;
pub const OFF_CMD: u32 = 0x0C;
pub const OFF_ARG0: u32 = 0x10;
pub const OFF_ARG1: u32 = 0x14;
pub const OFF_STATUS: u32 = 0x18;
pub const OFF_RESP: u32 = 0x1C;

// Status values.
pub const STATUS_IDLE: u32 = 0;
pub const STATUS_BUSY: u32 = 1;
pub const STATUS_DONE: u32 = 2;
pub const STATUS_ERR: u32 = 3;

// Command ids.
pub const CMD_PING: u32 = 0;
pub const CMD_SET_GPIO: u32 = 1;
pub const CMD_READ_GPIO: u32 = 2;
pub const CMD_READ_REG: u32 = 3;
pub const CMD_UART_TX: u32 = 4;
pub const CMD_UART_RX_POLL: u32 = 5;

/// `UART_RX_POLL` resp sentinel meaning "no byte available".
pub const RX_NONE: u32 = 0xFFFF_FFFF;

/// Encode a GPIO port+pin into a command argument: `(port_index << 8) | pin`.
pub fn encode_pin(port: nucleus_db::Port, pin: u8) -> u32 {
    ((port as u32) << 8) | (pin as u32)
}
