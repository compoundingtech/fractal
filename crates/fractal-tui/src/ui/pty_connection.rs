//! Viewer-side framing keeps the initial snapshot's geometry at its wire boundary.
//! SessionConnection's effective dimensions include read-ahead frames after SCREEN.
use pty_client::connection::SessionEvent;
use pty_core::protocol::{self, MessageType, PacketReader};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

pub(super) struct Connection {
    socket: UnixStream,
    reader: PacketReader,
    pending: VecDeque<SessionEvent>,
    screen: Vec<u8>,
    rows: u16,
    cols: u16,
}

impl Connection {
    pub(super) fn attach_over(
        mut socket: UnixStream,
        rows: u16,
        cols: u16,
        timeout: Duration,
    ) -> io::Result<Self> {
        socket.write_all(&protocol::encode_attach(rows, cols))?;
        let mut connection = Self {
            socket,
            reader: PacketReader::new(),
            pending: VecDeque::new(),
            screen: Vec::new(),
            rows,
            cols,
        };
        let deadline = Instant::now() + timeout;
        let mut before_screen = VecDeque::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "terminal attachment timed out"));
            }
            match connection.next_event(Some(remaining))? {
                Some(SessionEvent::Geometry { rows, cols }) => {
                    connection.rows = rows.max(1);
                    connection.cols = cols.max(1);
                }
                Some(SessionEvent::Screen(bytes)) => {
                    connection.screen = bytes;
                    before_screen.append(&mut connection.pending);
                    connection.pending = before_screen;
                    return Ok(connection);
                }
                Some(SessionEvent::Closed | SessionEvent::Exit(_)) => {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "terminal closed before screen"));
                }
                Some(event) => before_screen.push_back(event),
                None => {}
            }
        }
    }

    // Snapshot dimensions advance only as attach consumes events, never on read-ahead.
    pub(super) fn effective_rows(&self) -> u16 { self.rows }
    pub(super) fn effective_cols(&self) -> u16 { self.cols }
    pub(super) fn screen(&self) -> &[u8] { &self.screen }

    pub(super) fn next_event(&mut self, timeout: Option<Duration>) -> io::Result<Option<SessionEvent>> {
        if let Some(event) = self.pending.pop_front() {
            return Ok(Some(event));
        }
        self.socket.set_read_timeout(timeout)?;
        let mut bytes = [0; 16384];
        let count = match self.socket.read(&mut bytes) {
            Ok(0) => return Ok(Some(SessionEvent::Closed)),
            Ok(count) => count,
            Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let packets = self.reader.feed(&bytes[..count])
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        for packet in packets {
            let event = match packet.type_ {
                MessageType::Geometry => {
                    let (rows, cols) = protocol::decode_geometry(&packet.payload);
                    SessionEvent::Geometry { rows, cols }
                }
                MessageType::Screen => SessionEvent::Screen(packet.payload),
                MessageType::Data => SessionEvent::Data(packet.payload),
                MessageType::Exit => SessionEvent::Exit(protocol::decode_exit(&packet.payload)),
                _ => continue,
            };
            self.pending.push_back(event);
        }
        Ok(self.pending.pop_front())
    }

    pub(super) fn write(&mut self, bytes: &[u8]) {
        let _ = self.socket.write_all(&protocol::encode_data(bytes));
    }

    pub(super) fn resize(&mut self, rows: u16, cols: u16) {
        let _ = self.socket.write_all(&protocol::encode_resize(rows, cols));
    }

    pub(super) fn reset_input_modes(&mut self) {
        let _ = self.socket.write_all(&protocol::encode_packet(MessageType::ResetInputModes, &[]));
    }

    pub(super) fn disconnect(&mut self) {
        let _ = self.socket.write_all(&protocol::encode_detach());
        let _ = self.socket.shutdown(std::net::Shutdown::Both);
    }
}

impl Drop for Connection {
    fn drop(&mut self) { self.disconnect(); }
}
