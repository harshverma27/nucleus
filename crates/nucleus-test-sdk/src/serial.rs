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
            Serial::Tcp(s) => { s.write_all(&[b])?; s.flush() }
            Serial::Port(p) => { p.write_all(&[b])?; p.flush() }
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
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => Ok(None),
                    Err(e) => Err(e),
                }
            }
            Serial::Port(p) => {
                p.set_timeout(timeout).ok();
                match p.read(&mut buf) {
                    Ok(0) => Ok(None),
                    Ok(_) => Ok(Some(buf[0])),
                    Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(None),
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
        let _server = std::thread::spawn(move || { let _ = listener.accept(); std::thread::sleep(Duration::from_millis(200)); });
        let mut s = Serial::open_tcp(&addr.to_string()).unwrap();
        assert_eq!(s.read_byte(Duration::from_millis(50)).unwrap(), None);
    }
}
