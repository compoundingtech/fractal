#!/usr/bin/env python3
"""Quitting leaves no terminal input behind for the shell to print.

Usage: python3 crates/fractal-tui/tests/exit_leftover_probe.py target/debug/st3

Run `st3 tui --demo` on a pty, act as the terminal, press Ctrl+Q while mouse reports are still
arriving, and report the input bytes left unread after the process exits (what the shell would print).

The "terminal" here keeps reporting mouse moves until it sees the mouse-capture disable sequence,
as a real one does; a status-report query (CSI 5 n) is answered in order, after what it already sent."""
import fcntl, os, select, struct, subprocess, sys, termios, time

binary = sys.argv[1]
master, slave = os.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 140, 0, 0))

def child():
    os.setsid()
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)

proc = subprocess.Popen([binary, "tui", "--demo"], stdin=slave, stdout=slave, stderr=slave,
                        preexec_fn=child, close_fds=True)
out = b""
mouse_on = True

def pump(seconds, stop_when_exited=False):
    global out, mouse_on
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([master], [], [], 0.02)
        if r:
            data = os.read(master, 65536)
            out += data
            if b"\x1b[c" in data:
                os.write(master, b"\x1b[?62;c")
            if b"\x1b[?1006l" in data or b"\x1b[?1000l" in data:
                mouse_on = False
            if b"\x1b[5n" in data:
                os.write(master, b"\x1b[0n")
        if stop_when_exited and proc.poll() is not None:
            return

pump(2.0)
os.write(master, b"\x11")  # Ctrl+Q
# The terminal keeps reporting moves until it has read the disable sequence the TUI sends on exit.
n = 0
t0 = time.time()
while mouse_on and time.time() - t0 < 3:
    os.write(master, b"\x1b[<35;%d;10M" % (20 + n % 50)); n += 1
    r, _, _ = select.select([master], [], [], 0.0005)
    if r:
        data = os.read(master, 65536); out += data
        if b"\x1b[c" in data: os.write(master, b"\x1b[?62;c")
        if b"\x1b[?1006l" in data or b"\x1b[?1000l" in data: mouse_on = False
        if b"\x1b[5n" in data: os.write(master, b"\x1b[0n")
os.write(master, b"\x1b[O")  # a focus change that was already on its way
pump(3.0, stop_when_exited=True)
print("exited:", proc.poll())

# Whatever is still queued for the slave is what the shell would read.
os.set_blocking(slave, False)
attrs = termios.tcgetattr(slave)
raw = attrs[:]
raw[3] &= ~(termios.ICANON | termios.ECHO)
termios.tcsetattr(slave, termios.TCSANOW, raw)
time.sleep(0.2)
try:
    left = os.read(slave, 65536)
except BlockingIOError:
    left = b""
print("left unread:", left)
print("sent 5n:", b"\x1b[5n" in out)
sys.exit(1 if left else 0)
