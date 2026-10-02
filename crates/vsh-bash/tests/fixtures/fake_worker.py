#!/usr/bin/python3
"""Deliberately invalid worker for bounded transport teardown tests."""

import os
import struct
import sys
import time
from pathlib import Path

mode = Path(sys.argv[0]).stem
worker_id = b"vsh-bash-worker/4 bashkit/0.18.2 vsh/0.5.0"
output = sys.stdout.buffer
input_stream = sys.stdin.buffer

if mode == "bad_version":
    worker_id = b"wrong-worker"

hello = bytes([1]) + struct.pack("<QQQ", 0, 0, len(worker_id)) + worker_id
output.write(struct.pack("<I", len(hello)) + hello)
output.flush()

if mode == "bad_version":
    time.sleep(10)

if mode == "blocked_write":
    time.sleep(10)

header = input_stream.read(4)
if len(header) != 4:
    sys.exit(0)
length = struct.unpack("<I", header)[0]
run = input_stream.read(length)
session = struct.unpack("<Q", run[1:9])[0]

if mode == "oversized_frame":
    output.write(struct.pack("<I", 0xFFFFFFFF))
elif mode == "truncated_frame":
    output.write(struct.pack("<I", 100) + b"x")
elif mode == "bad_nested_length":
    call = (
        bytes([3])
        + struct.pack("<QQ", session, 1)
        + bytes([1])
        + struct.pack("<Q", 0xFFFFFFFFFFFFFFFF)
    )
    output.write(struct.pack("<I", len(call)) + call)
elif mode == "wrong_direction":
    reply = bytes([4]) + struct.pack("<QQ", session, 1) + bytes([0, 0])
    output.write(struct.pack("<I", len(reply)) + reply)
else:
    if mode == "stderr_flood":
        os.write(2, b"diagnostic flood\n" * 10000)
    done = bytes([5]) + struct.pack("<QQiQQB", session + (mode == "wrong_session"), 1, 0, 0, 0, 0)
    output.write(struct.pack("<I", len(done)) + done)
    if mode == "double_finish":
        output.write(struct.pack("<I", len(done)) + done)
    elif mode == "call_after_finish":
        call = (
            bytes([3]) + struct.pack("<QQ", session, 1) + bytes([6]) + struct.pack("<Q", 1) + b"."
        )
        output.write(struct.pack("<I", len(call)) + call)
    else:
        ready = bytes([6]) + struct.pack("<QQ", session, 1)
        output.write(struct.pack("<I", len(ready)) + ready)
output.flush()

if mode != "truncated_frame":
    time.sleep(10)
