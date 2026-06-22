use std::time::{Duration, Instant};

use nucleus_db::Port;
use nucleus_hil::backend::{Backend, HilError};

use crate::protocol::*;

/// Errors talking to the device test-agent.
#[derive(Debug)]
pub enum SdkError {
    Hil(HilError),
    BadMagic(u32),
    VersionMismatch { found: u32, expected: u32 },
    AgentError { cmd: u32 },
    Timeout { cmd: u32 },
}

impl std::fmt::Display for SdkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SdkError::Hil(e) => write!(f, "backend error: {e}"),
            SdkError::BadMagic(m) => write!(f, "mailbox magic mismatch: got 0x{m:08x}"),
            SdkError::VersionMismatch { found, expected } => {
                write!(f, "agent protocol v{found}, SDK expects v{expected}")
            }
            SdkError::AgentError { cmd } => write!(f, "agent reported error for command {cmd}"),
            SdkError::Timeout { cmd } => write!(f, "agent did not complete command {cmd} in time"),
        }
    }
}
impl std::error::Error for SdkError {}
impl From<HilError> for SdkError {
    fn from(e: HilError) -> Self {
        SdkError::Hil(e)
    }
}

/// Host-side driver for the on-device test-agent's RAM mailbox.
pub struct AgentClient<'a> {
    backend: &'a mut dyn Backend,
    base: u32,
    poll_timeout: Duration,
}

impl<'a> AgentClient<'a> {
    pub fn new(backend: &'a mut dyn Backend) -> Self {
        Self {
            backend,
            base: MAILBOX_BASE,
            poll_timeout: Duration::from_millis(500),
        }
    }

    /// Verify the agent is alive: poll for the magic word, then read version.
    ///
    /// The magic is published by the agent only after it finishes booting
    /// (clocks/USART/GPIO init), and the agent writes it LAST so a non-zero
    /// magic always implies a fully-initialized mailbox. A bare single read
    /// races that boot — on a backend that halts the target the instant the
    /// host attaches (QEMU's `-S` stub), the first read can land before the
    /// agent has run far enough to set the magic, yielding 0. So poll up to
    /// `poll_timeout` for the magic to appear; a still-zero magic past the
    /// deadline is reported as `BadMagic`, a genuinely wrong magic fails fast.
    pub fn connect(&mut self) -> Result<u32, SdkError> {
        let deadline = Instant::now() + self.poll_timeout;
        loop {
            let magic = self.backend.read_mem32(self.base + OFF_MAGIC)?;
            if magic == MAGIC {
                break;
            }
            // A non-zero, non-matching magic is a real mismatch, not a boot
            // race — fail immediately rather than waiting out the timeout.
            if magic != 0 || Instant::now() >= deadline {
                return Err(SdkError::BadMagic(magic));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let version = self.backend.read_mem32(self.base + OFF_VERSION)?;
        if version != PROTO_VERSION {
            return Err(SdkError::VersionMismatch {
                found: version,
                expected: PROTO_VERSION,
            });
        }
        Ok(version)
    }

    /// Issue one command and block until the agent posts DONE/ERR (or times out).
    fn issue(&mut self, cmd: u32, arg0: u32, arg1: u32) -> Result<u32, SdkError> {
        self.backend.write_mem32(self.base + OFF_ARG0, arg0)?;
        self.backend.write_mem32(self.base + OFF_ARG1, arg1)?;
        self.backend.write_mem32(self.base + OFF_CMD, cmd)?;
        let seq = self.backend.read_mem32(self.base + OFF_SEQ)?;
        self.backend
            .write_mem32(self.base + OFF_SEQ, seq.wrapping_add(1))?;
        self.backend
            .write_mem32(self.base + OFF_STATUS, STATUS_BUSY)?;

        let deadline = Instant::now() + self.poll_timeout;
        loop {
            let status = self.backend.read_mem32(self.base + OFF_STATUS)?;
            match status {
                STATUS_DONE => return Ok(self.backend.read_mem32(self.base + OFF_RESP)?),
                STATUS_ERR => return Err(SdkError::AgentError { cmd }),
                _ if Instant::now() >= deadline => return Err(SdkError::Timeout { cmd }),
                _ => std::thread::sleep(Duration::from_millis(2)),
            }
        }
    }

    pub fn ping(&mut self) -> Result<u32, SdkError> {
        self.issue(CMD_PING, 0, 0)
    }

    pub fn set_gpio(&mut self, port: Port, pin: u8, level: bool) -> Result<(), SdkError> {
        self.issue(CMD_SET_GPIO, encode_pin(port, pin), u32::from(level))
            .map(|_| ())
    }

    pub fn read_gpio(&mut self, port: Port, pin: u8) -> Result<bool, SdkError> {
        Ok(self.issue(CMD_READ_GPIO, encode_pin(port, pin), 0)? != 0)
    }

    pub fn read_register(&mut self, addr: u32) -> Result<u32, SdkError> {
        self.issue(CMD_READ_REG, addr, 0)
    }

    pub fn uart_tx(&mut self, byte: u8) -> Result<(), SdkError> {
        self.issue(CMD_UART_TX, u32::from(byte), 0).map(|_| ())
    }

    pub fn uart_rx_poll(&mut self) -> Result<Option<u8>, SdkError> {
        let r = self.issue(CMD_UART_RX_POLL, 0, 0)?;
        Ok(if r == RX_NONE { None } else { Some(r as u8) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nucleus_hil::backend::{
        Backend, BackendKind, FirmwareArtifact, HilError, ItmEvent, RunResult, Sample,
    };
    use std::collections::HashMap;
    use std::time::Duration;

    /// RAM + a simulated agent. On any write that sets STATUS=BUSY, runs the
    /// command and posts DONE/resp, so the host handshake can be tested with
    /// no real device.
    struct FakeDevice {
        ram: HashMap<u32, u32>,
        gpio: HashMap<u32, bool>, // keyed by encoded pin
        rx_queue: std::collections::VecDeque<u8>,
        tx_log: Vec<u8>,
    }

    impl FakeDevice {
        fn new() -> Self {
            let mut ram = HashMap::new();
            ram.insert(MAILBOX_BASE + OFF_MAGIC, MAGIC);
            ram.insert(MAILBOX_BASE + OFF_VERSION, PROTO_VERSION);
            ram.insert(MAILBOX_BASE + OFF_STATUS, STATUS_IDLE);
            Self {
                ram,
                gpio: HashMap::new(),
                rx_queue: Default::default(),
                tx_log: Vec::new(),
            }
        }
        fn run_command(&mut self) {
            let cmd = *self.ram.get(&(MAILBOX_BASE + OFF_CMD)).unwrap_or(&0);
            let a0 = *self.ram.get(&(MAILBOX_BASE + OFF_ARG0)).unwrap_or(&0);
            let mut resp = 0u32;
            match cmd {
                CMD_PING => resp = PROTO_VERSION,
                CMD_SET_GPIO => {
                    let level = *self.ram.get(&(MAILBOX_BASE + OFF_ARG1)).unwrap_or(&0) != 0;
                    self.gpio.insert(a0, level);
                }
                CMD_READ_GPIO => resp = u32::from(*self.gpio.get(&a0).unwrap_or(&false)),
                CMD_READ_REG => resp = *self.ram.get(&a0).unwrap_or(&0),
                CMD_UART_TX => self.tx_log.push(a0 as u8),
                CMD_UART_RX_POLL => {
                    resp = self.rx_queue.pop_front().map(u32::from).unwrap_or(RX_NONE)
                }
                _ => {
                    self.ram.insert(MAILBOX_BASE + OFF_RESP, 0);
                    self.ram.insert(MAILBOX_BASE + OFF_STATUS, STATUS_ERR);
                    return;
                }
            }
            self.ram.insert(MAILBOX_BASE + OFF_RESP, resp);
            self.ram.insert(MAILBOX_BASE + OFF_STATUS, STATUS_DONE);
        }
    }

    impl Backend for FakeDevice {
        fn name(&self) -> BackendKind {
            BackendKind::Qemu
        }
        fn start(
            &mut self,
            _f: &FirmwareArtifact,
            _r: &nucleus_compiler::CheckReport,
        ) -> Result<(), HilError> {
            Ok(())
        }
        fn pin(&mut self, _p: nucleus_db::Port, _n: u8) -> Result<bool, HilError> {
            Ok(false)
        }
        fn register(&mut self, _p: &str, _o: u32) -> Result<u32, HilError> {
            Ok(0)
        }
        fn await_itm_event(&mut self, _t: Duration) -> Result<Option<ItmEvent>, HilError> {
            Ok(None)
        }
        fn sample(&mut self, _d: Duration) -> Result<Sample, HilError> {
            unimplemented!()
        }
        fn finish(&mut self) -> RunResult {
            unimplemented!()
        }
        fn read_mem32(&mut self, addr: u32) -> Result<u32, HilError> {
            Ok(*self.ram.get(&addr).unwrap_or(&0))
        }
        fn write_mem32(&mut self, addr: u32, value: u32) -> Result<(), HilError> {
            self.ram.insert(addr, value);
            if addr == MAILBOX_BASE + OFF_STATUS && value == STATUS_BUSY {
                self.run_command();
            }
            Ok(())
        }
    }

    #[test]
    fn connect_verifies_magic_and_version() {
        let mut dev = FakeDevice::new();
        let mut c = AgentClient::new(&mut dev);
        assert_eq!(c.connect().unwrap(), PROTO_VERSION);
    }

    #[test]
    fn connect_rejects_version_mismatch() {
        let mut dev = FakeDevice::new();
        dev.ram.insert(MAILBOX_BASE + OFF_VERSION, 99);
        let mut c = AgentClient::new(&mut dev);
        assert!(matches!(c.connect(), Err(SdkError::VersionMismatch { .. })));
    }

    #[test]
    fn ping_returns_version() {
        let mut dev = FakeDevice::new();
        let mut c = AgentClient::new(&mut dev);
        c.connect().unwrap();
        assert_eq!(c.ping().unwrap(), PROTO_VERSION);
    }

    #[test]
    fn set_then_read_gpio_roundtrips() {
        let mut dev = FakeDevice::new();
        let mut c = AgentClient::new(&mut dev);
        c.connect().unwrap();
        c.set_gpio(nucleus_db::Port::A, 5, true).unwrap();
        assert!(c.read_gpio(nucleus_db::Port::A, 5).unwrap());
    }

    #[test]
    fn uart_rx_poll_returns_none_then_byte() {
        let mut dev = FakeDevice::new();
        dev.rx_queue.push_back(0x42);
        let mut c = AgentClient::new(&mut dev);
        c.connect().unwrap();
        assert_eq!(c.uart_rx_poll().unwrap(), Some(0x42));
        assert_eq!(c.uart_rx_poll().unwrap(), None);
    }

    #[test]
    fn uart_tx_reaches_device() {
        let mut dev = FakeDevice::new();
        {
            let mut c = AgentClient::new(&mut dev);
            c.connect().unwrap();
            c.uart_tx(0x5A).unwrap();
        }
        assert_eq!(dev.tx_log, vec![0x5A]);
    }
}
