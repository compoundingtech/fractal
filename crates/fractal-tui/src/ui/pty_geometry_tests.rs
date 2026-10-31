use super::*;
use pty_core::protocol::{
    MessageType, Packet, PacketReader, decode_cell, decode_size, encode_geometry, encode_packet,
};
use std::collections::VecDeque;
use std::io::{Read as _, Write as _};

fn next_packet(
    daemon: &mut UnixStream,
    reader: &mut PacketReader,
    pending: &mut VecDeque<Packet>,
) -> Packet {
    loop {
        if let Some(packet) = pending.pop_front() {
            return packet;
        }
        let mut bytes = [0; 256];
        let count = daemon.read(&mut bytes).unwrap();
        assert_ne!(count, 0, "terminal disconnected before sending a packet");
        pending.extend(reader.feed(&bytes[..count]).unwrap());
    }
}

fn wait_for(done: impl Fn() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(5), "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn visible_rows(screen: &Screen) -> Vec<String> {
    (0..screen.term.screen_lines())
        .map(|row| {
            (0..screen.term.columns())
                .map(|column| {
                    screen.term.grid()[Point::new(GridLine(row as i32), Column(column))].c
                })
                .collect()
        })
        .collect()
}

#[test]
fn coalesced_initial_snapshot_uses_its_geometry_before_a_later_shrink() {
    let (tui, mut daemon) = UnixStream::pair().unwrap();
    daemon.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let snapshot = b"\x1b[?7l\x1b[6;1H\r$1.00\x1b[7;1H\reditor\x1b[8;1H\rBOTTOM";
    let barrier = b"\x1b]2;coalesced-initial-geometry\x07";
    let mut coalesced = encode_geometry(8, 20);
    coalesced.extend(encode_packet(MessageType::Screen, snapshot));
    coalesced.extend(encode_geometry(7, 10));
    coalesced.extend(encode_packet(MessageType::Data, barrier));
    // Queue the entire stream before the attach thread can read it: the first
    // socket read sees both geometries, not just the one belonging to SCREEN.
    assert_eq!(daemon.write(&coalesced).unwrap(), coalesced.len());
    let terminal = NativeTerminal::spawn(tui, "test", "one".into(), 10, 30);
    let mut reader = PacketReader::new();
    let mut pending = VecDeque::new();
    assert_eq!(
        next_packet(&mut daemon, &mut reader, &mut pending).type_,
        MessageType::Attach
    );
    wait_for(|| terminal.title().as_deref() == Some("coalesced-initial-geometry"));

    // Use the same Alacritty emulator as a control, explicitly preserving the
    // snapshot's bottom-row cursor before the subsequent height shrink.
    let mut control = Screen::new(
        Size {
            rows: 8,
            columns: 20,
        },
        Requests::default(),
    );
    control.feed(snapshot);
    assert_eq!(control.term.grid().cursor.point, Point::new(GridLine(7), Column(6)));
    assert_eq!(control.term.grid()[Point::new(GridLine(7), Column(0))].c, 'B');
    control.term.resize(Size {
        rows: 7,
        columns: 10,
    });
    control.feed(barrier);

    // Prove that this fixture distinguishes replay at the read-ahead size;
    // merely checking the final dimensions would accept that broken ordering.
    let mut read_ahead = Screen::new(
        Size {
            rows: 7,
            columns: 10,
        },
        Requests::default(),
    );
    read_ahead.feed(snapshot);
    assert_ne!(visible_rows(&control), visible_rows(&read_ahead));

    let screen = terminal.lock();
    assert!(screen.attached);
    assert_eq!((screen.term.screen_lines(), screen.term.columns()), (7, 10));
    assert_eq!(screen.term.grid().cursor.point, control.term.grid().cursor.point);
    assert_eq!(visible_rows(&screen), visible_rows(&control));
}

#[test]
fn cell_metrics_preserve_requested_dimensions_under_shared_geometry_constraints() {
    let (tui, mut daemon) = UnixStream::pair().unwrap();
    daemon.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let terminal = NativeTerminal::spawn(tui, "test", "one".into(), 10, 30);
    let mut reader = PacketReader::new();
    let mut pending = VecDeque::new();
    let attach = next_packet(&mut daemon, &mut reader, &mut pending);
    assert_eq!(attach.type_, MessageType::Attach);
    assert_eq!(decode_size(&attach.payload), (10, 30));

    let mut initial = encode_geometry(8, 20);
    initial.extend(encode_packet(MessageType::Screen, b""));
    daemon.write_all(&initial).unwrap();
    wait_for(|| terminal.attached());
    assert_eq!(terminal.grid_size(), (8, 20));
    terminal.send(Input::Cell(pty_terminal::CellSize {
        width: 8,
        height: 16,
    }));
    let cell_resize = next_packet(&mut daemon, &mut reader, &mut pending);
    assert_eq!(cell_resize.type_, MessageType::Resize);
    assert_eq!(decode_size(&cell_resize.payload), (10, 30));
    assert_eq!(decode_cell(&cell_resize.payload), Some((8, 16)));

    terminal.fit(7, 25);
    let resize = next_packet(&mut daemon, &mut reader, &mut pending);
    assert_eq!(resize.type_, MessageType::Resize);
    assert_eq!(decode_size(&resize.payload), (7, 25));
    let mut constrained = encode_geometry(6, 20);
    constrained.extend(encode_packet(
        MessageType::Data,
        b"\x1b]2;constrained-after-fit\x07",
    ));
    daemon.write_all(&constrained).unwrap();
    wait_for(|| terminal.title().as_deref() == Some("constrained-after-fit"));
    assert_eq!(terminal.grid_size(), (6, 20));
    terminal.send(Input::Cell(pty_terminal::CellSize {
        width: 8,
        height: 16,
    }));
    let cell_resize = next_packet(&mut daemon, &mut reader, &mut pending);
    assert_eq!(cell_resize.type_, MessageType::Resize);
    assert_eq!(decode_size(&cell_resize.payload), (7, 25));
    assert_eq!(decode_cell(&cell_resize.payload), Some((8, 16)));
}
