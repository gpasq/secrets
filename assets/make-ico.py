"""Pack PNG files into a Windows .ico (PNG-compressed entries, Vista+).

usage: python3 make-ico.py out.ico 16.png 32.png 48.png 256.png ...
"""
import struct
import sys


def png_size(data: bytes) -> tuple[int, int]:
    return struct.unpack(">II", data[16:24])


def main(out: str, pngs: list[str]) -> None:
    images = [open(p, "rb").read() for p in pngs]
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries = b""
    for data in images:
        w, h = png_size(data)
        entries += struct.pack(
            "<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(data), offset
        )
        offset += len(data)
    with open(out, "wb") as f:
        f.write(header + entries + b"".join(images))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2:])
