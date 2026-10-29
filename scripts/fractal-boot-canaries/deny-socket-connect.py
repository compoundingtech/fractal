#!/usr/bin/env python3
"""Deny connect(2) in this one CLI process with Linux seccomp, then exec it.

The provider and private daemon remain outside this filter. This produces a real
EPERM on the daemon Unix socket without a credential or a provider model.
"""
import ctypes
import os
import platform
import sys

if platform.system() != "Linux" or platform.machine() != "x86_64":
    raise SystemExit("the socket-denial fixture requires Linux x86_64")


class Filter(ctypes.Structure):
    _fields_ = [("code", ctypes.c_ushort), ("jt", ctypes.c_ubyte),
                ("jf", ctypes.c_ubyte), ("k", ctypes.c_uint)]


class Program(ctypes.Structure):
    _fields_ = [("len", ctypes.c_ushort), ("filter", ctypes.POINTER(Filter))]


filters = (Filter * 4)(Filter(0x20, 0, 0, 0), Filter(0x15, 0, 1, 42),
                       Filter(0x06, 0, 0, 0x00050001), Filter(0x06, 0, 0, 0x7fff0000))
program = Program(len(filters), filters)
libc = ctypes.CDLL(None, use_errno=True)
if libc.prctl(38, 1, 0, 0, 0) or libc.prctl(22, 2, ctypes.byref(program), 0, 0):
    raise OSError(ctypes.get_errno(), "installing the test-only seccomp filter")
os.execv(sys.argv[1], sys.argv[1:])
