#!/usr/bin/env python3
"""Prove that both exit controls leave a live attached terminal without input to it: Ctrl+\\
and a click on the terminal's header line, in Fractal TUI's spaces. Back in the conversation it
attaches again by the keys the conversation names (Ctrl+A then t), not by an undrawn control."""

import sys
import time

sys.dont_write_bytecode = True
from interaction_qa import click, open_agent, screen, send, start, stop, wait_screen

HEADER = "← Ctrl+\\"  # the attached terminal's header line
DETACHED = "Not attached. Enter attaches"  # a detached terminal pane, the one place Attach is drawn
TERMINAL_ROW = " t  Terminal"  # the agent actions menu's Terminal row


def attached(value: str) -> bool:
    return HEADER in value


def attach(session) -> None:
    """Attach by what the screen offers: the Attach button on a detached terminal pane, else from
    the conversation by the keys its guidance names, Ctrl+A (the agent's actions) then t."""
    value = screen(session)
    if attached(value):
        return
    lines = value.splitlines()
    detached = next((index for index, line in enumerate(lines) if DETACHED in line), None)
    if detached is not None:
        row = next(index for index in range(detached + 1, len(lines)) if "Attach" in lines[index])
        click(session, lines[row].index("Attach") + 2, row + 1)
        return
    send(session, "\x01")  # Ctrl+A: the agent's actions
    wait_screen(session, lambda text: TERMINAL_ROW in text, "actions menu Terminal row")
    send(session, "t")


def main(binary: str, name: str) -> None:
    session = start(binary, "stui-attach-qa")
    try:
        open_agent(session, name)
        for _ in range(3):
            attach(session)
            try:
                wait_screen(session, attached, "attached terminal", seconds=5)
                break
            except AssertionError:
                time.sleep(1)
        else:
            raise AssertionError(f"attach failed for {name}")
        send(session, "\x1c")  # Ctrl+\, the leader
        send(session, "c")     # then c: the conversation
        wait_screen(session, lambda value: not attached(value), "Ctrl+\\ c detach")
        attach(session)
        value = wait_screen(session, attached, "attached again")
        row = next(index for index, line in enumerate(value.splitlines()) if HEADER in line)
        column = value.splitlines()[row].index(HEADER)
        click(session, column + 2, row + 1)
        wait_screen(session, lambda value: not attached(value), "click on the header detaches")
        print(f"Attachment QA passed for {name}: Ctrl+\\ and a click on the header detach")
    finally:
        stop(session)


if __name__ == "__main__":
    if len(sys.argv) < 3:
        raise SystemExit("usage: attachment_qa.py STUI AGENT_NAME")
    main(sys.argv[1], sys.argv[2])
