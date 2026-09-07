#!/usr/bin/env python3
"""Reject empty/corrupt cross-platform release binaries before packaging."""

from pathlib import Path
import sys


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: validate_binary.py <path> <target>", file=sys.stderr)
        return 2

    path = Path(sys.argv[1])
    target = sys.argv[2]
    if not path.is_file() or path.stat().st_size < 1024:
        print(f"error: missing or implausibly small binary: {path}", file=sys.stderr)
        return 1

    data = path.read_bytes()
    if not data.strip(b"\0"):
        print(f"error: binary contains only zero bytes: {path}", file=sys.stderr)
        return 1

    magic = data[:4]
    if "windows" in target:
        valid = data[:2] == b"MZ"
        expected = "PE/COFF (MZ)"
    elif "apple" in target:
        valid = magic in {
            b"\xcf\xfa\xed\xfe",  # Mach-O 64, little-endian
            b"\xfe\xed\xfa\xcf",  # Mach-O 64, big-endian
            b"\xca\xfe\xba\xbe",  # universal/fat, big-endian
            b"\xbe\xba\xfe\xca",  # universal/fat, little-endian
        }
        expected = "Mach-O"
    elif "linux" in target:
        valid = magic == b"\x7fELF"
        expected = "ELF"
    else:
        print(f"error: unsupported validation target: {target}", file=sys.stderr)
        return 2

    if not valid:
        print(
            f"error: {path} is not {expected}; magic={data[:16].hex()} size={len(data)}",
            file=sys.stderr,
        )
        return 1

    print(f"validated {expected}: {path} ({len(data)} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
