//! Host-side UART access for scripted tests: the ST-Link Virtual COM Port on
//! real hardware (`/dev/ttyACM*`), or QEMU's `-serial tcp` socket on the
//! emulator. One byte at a time, with a read timeout that degrades to
//! `Ok(None)` rather than erroring.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub enum Serial {
    Tcp(TcpStream),
    Port(Box<dyn serialport::SerialPort>),
}

impl Serial {
    pub fn open_tcp(addr: &str) -> std::io::Result<Self> {
        let stream = TcpStream::connect(addr)?;
        Ok(Serial::Tcp(stream))
    }

    pub fn open_device(path: &str, baud: u32) -> Result<Self, serialport::Error> {
        let port = serialport::new(path, baud)
            .timeout(Duration::from_millis(50))
            .open()?;
        Ok(Serial::Port(port))
    }

    pub fn write_byte(&mut self, b: u8) -> std::io::Result<()> {
        match self {
            Serial::Tcp(s) => {
                s.write_all(&[b])?;
                s.flush()
            }
            Serial::Port(p) => {
                p.write_all(&[b])?;
                p.flush()
            }
        }
    }

    /// Read one byte, waiting up to `timeout`. `Ok(None)` means nothing
    /// arrived in time (not an error).
    pub fn read_byte(&mut self, timeout: Duration) -> std::io::Result<Option<u8>> {
        let mut buf = [0u8; 1];
        match self {
            Serial::Tcp(s) => {
                s.set_read_timeout(Some(timeout))?;
                match s.read(&mut buf) {
                    Ok(0) => Ok(None),
                    Ok(_) => Ok(Some(buf[0])),
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        Ok(None)
                    }
                    Err(e) => Err(e),
                }
            }
            Serial::Port(p) => {
                p.set_timeout(timeout).ok();
                match p.read(&mut buf) {
                    Ok(0) => Ok(None),
                    Ok(_) => Ok(Some(buf[0])),
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        Ok(None)
                    }
                    Err(e) => Err(e),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    #[test]
    fn tcp_roundtrips_a_byte() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            // echo one byte back
            let mut buf = [0u8; 1];
            use std::io::Read;
            sock.read_exact(&mut buf).unwrap();
            sock.write_all(&buf).unwrap();
        });
        let mut s = Serial::open_tcp(&addr.to_string()).unwrap();
        s.write_byte(0x39).unwrap();
        assert_eq!(s.read_byte(Duration::from_secs(1)).unwrap(), Some(0x39));
        server.join().unwrap();
    }

    #[test]
    fn read_byte_times_out_to_none() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let _server = std::thread::spawn(move || {
            let _ = listener.accept();
            std::thread::sleep(Duration::from_millis(200));
        });
        let mut s = Serial::open_tcp(&addr.to_string()).unwrap();
        assert_eq!(s.read_byte(Duration::from_millis(50)).unwrap(), None);
    }

    /// A `serialport::SerialPort` stub whose `read` always returns
    /// `ErrorKind::WouldBlock` — some serialport backends report a timed-out
    /// read this way instead of `ErrorKind::TimedOut`.
    struct WouldBlockPort;

    impl std::io::Read for WouldBlockPort {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(std::io::ErrorKind::WouldBlock))
        }
    }

    impl std::io::Write for WouldBlockPort {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl serialport::SerialPort for WouldBlockPort {
        fn name(&self) -> Option<String> {
            None
        }
        fn baud_rate(&self) -> serialport::Result<u32> {
            Ok(115200)
        }
        fn data_bits(&self) -> serialport::Result<serialport::DataBits> {
            Ok(serialport::DataBits::Eight)
        }
        fn flow_control(&self) -> serialport::Result<serialport::FlowControl> {
            Ok(serialport::FlowControl::None)
        }
        fn parity(&self) -> serialport::Result<serialport::Parity> {
            Ok(serialport::Parity::None)
        }
        fn stop_bits(&self) -> serialport::Result<serialport::StopBits> {
            Ok(serialport::StopBits::One)
        }
        fn timeout(&self) -> Duration {
            Duration::from_millis(50)
        }
        fn set_baud_rate(&mut self, _baud_rate: u32) -> serialport::Result<()> {
            Ok(())
        }
        fn set_data_bits(&mut self, _data_bits: serialport::DataBits) -> serialport::Result<()> {
            Ok(())
        }
        fn set_flow_control(
            &mut self,
            _flow_control: serialport::FlowControl,
        ) -> serialport::Result<()> {
            Ok(())
        }
        fn set_parity(&mut self, _parity: serialport::Parity) -> serialport::Result<()> {
            Ok(())
        }
        fn set_stop_bits(&mut self, _stop_bits: serialport::StopBits) -> serialport::Result<()> {
            Ok(())
        }
        fn set_timeout(&mut self, _timeout: Duration) -> serialport::Result<()> {
            Ok(())
        }
        fn write_request_to_send(&mut self, _level: bool) -> serialport::Result<()> {
            Ok(())
        }
        fn write_data_terminal_ready(&mut self, _level: bool) -> serialport::Result<()> {
            Ok(())
        }
        fn read_clear_to_send(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }
        fn read_data_set_ready(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }
        fn read_ring_indicator(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }
        fn read_carrier_detect(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }
        fn bytes_to_read(&self) -> serialport::Result<u32> {
            Ok(0)
        }
        fn bytes_to_write(&self) -> serialport::Result<u32> {
            Ok(0)
        }
        fn clear(&self, _buffer_to_clear: serialport::ClearBuffer) -> serialport::Result<()> {
            Ok(())
        }
        fn try_clone(&self) -> serialport::Result<Box<dyn serialport::SerialPort>> {
            Err(serialport::Error::new(
                serialport::ErrorKind::Unknown,
                "clone not supported in test stub",
            ))
        }
        fn set_break(&self) -> serialport::Result<()> {
            Ok(())
        }
        fn clear_break(&self) -> serialport::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn port_read_byte_treats_would_block_as_a_timeout_not_an_error() {
        // Issue #51: the `Port` branch only caught `ErrorKind::TimedOut`,
        // unlike the `Tcp` branch's `TimedOut | WouldBlock`. Some
        // serialport backends report a timed-out read as `WouldBlock`,
        // which used to surface as `Err` here instead of `Ok(None)`.
        let mut s = Serial::Port(Box::new(WouldBlockPort));
        assert_eq!(s.read_byte(Duration::from_millis(10)).unwrap(), None);
    }
}
