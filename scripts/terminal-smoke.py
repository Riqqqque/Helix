#!/usr/bin/env python3
"""Exercise helix-terminald over its private framed Unix-socket protocol."""

from __future__ import annotations

import argparse
import json
import socket
import struct
from pathlib import Path


PROTOCOL_VERSION = 1
CLIENT_OPEN = 1
CLIENT_INPUT = 2
CLIENT_RESIZE = 3
SERVER_READY = 101
SERVER_OUTPUT = 102
SERVER_EXIT = 103
SERVER_ERROR = 104
MAX_FRAME_BYTES = 64 * 1024


def send_frame(connection: socket.socket, kind: int, payload: bytes) -> None:
    length = len(payload) + 1
    if not 1 <= length <= MAX_FRAME_BYTES:
        raise RuntimeError("outgoing terminal frame is outside the protocol limit")
    connection.sendall(struct.pack(">I", length) + bytes([kind]) + payload)


def read_exact(connection: socket.socket, length: int) -> bytes:
    chunks = bytearray()
    while len(chunks) < length:
        chunk = connection.recv(length - len(chunks))
        if not chunk:
            raise RuntimeError("terminal daemon closed the socket mid-frame")
        chunks.extend(chunk)
    return bytes(chunks)


def read_frame(connection: socket.socket) -> tuple[int, bytes]:
    (length,) = struct.unpack(">I", read_exact(connection, 4))
    if not 1 <= length <= MAX_FRAME_BYTES:
        raise RuntimeError("terminal daemon returned an invalid frame length")
    body = read_exact(connection, length)
    return body[0], body[1:]


def run_smoke(socket_path: Path, expected_user: str) -> None:
    expected_marker = f"__HELIX_TERMINAL_SMOKE__:{expected_user}:".encode()
    expected_size = b"37 101"
    output = bytearray()

    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(10)
        connection.connect(str(socket_path))
        send_frame(
            connection,
            CLIENT_OPEN,
            json.dumps(
                {
                    "protocol_version": PROTOCOL_VERSION,
                    "dimensions": {"columns": 80, "rows": 24},
                },
                separators=(",", ":"),
            ).encode(),
        )
        kind, payload = read_frame(connection)
        if kind != SERVER_READY:
            raise RuntimeError(f"terminal daemon rejected the handshake (frame {kind})")
        ready = json.loads(payload)
        if ready.get("protocol_version") != PROTOCOL_VERSION:
            raise RuntimeError("terminal daemon reported a different protocol version")
        if ready.get("user") != expected_user:
            raise RuntimeError("terminal daemon reported the wrong operating-system user")

        send_frame(connection, CLIENT_RESIZE, struct.pack(">HH", 101, 37))
        send_frame(
            connection,
            CLIENT_INPUT,
            b'printf "__HELIX_%s_%s__:%s:%s\\n" "TERMINAL" "SMOKE" "$USER" "$PWD"; stty size; exit\n',
        )

        exit_code: int | None = None
        while exit_code is None:
            kind, payload = read_frame(connection)
            if kind == SERVER_OUTPUT:
                output.extend(payload)
                if len(output) > 256 * 1024:
                    raise RuntimeError("terminal smoke output exceeded its safety limit")
            elif kind == SERVER_EXIT:
                exit_code = int(json.loads(payload)["exit_code"])
            elif kind == SERVER_ERROR:
                raise RuntimeError("terminal daemon returned an error frame")
            else:
                raise RuntimeError(f"terminal daemon returned unexpected frame {kind}")

    if exit_code != 0:
        raise RuntimeError(f"terminal shell exited with code {exit_code}")
    if expected_marker not in output:
        raise RuntimeError("PTY output did not prove the expected user and working directory")
    if expected_size not in output:
        raise RuntimeError("PTY output did not reflect the requested terminal resize")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--socket", required=True, type=Path)
    parser.add_argument("--expected-user", required=True)
    args = parser.parse_args()
    run_smoke(args.socket, args.expected_user)
    print("terminal PTY smoke passed")


if __name__ == "__main__":
    main()
