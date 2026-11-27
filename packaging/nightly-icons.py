#!/usr/bin/env python3
"""Derive the Nightly app icons from the committed stable master, with no image tools needed.

The Nightly uses the same chameleon on a deeper field, so the two icons are told apart at 16 px
without a badge, crescent or letter. Nothing is upscaled: the 1024 px render (the real padded
macOS-grid master) is the only input, and every smaller frame is an area average of it, so each
size is rendered from the full detail.

The background contribution of every pixel is solved from the palette's three colours (ink and
paper are neutral, the field is saturated), the field colour is moved and the ink, paper, shading
and alpha are left exactly as they are.

    python3 packaging/nightly-icons.py [--check]

`--check` writes nothing and reports whether the committed files match what this script makes.
"""
import argparse
import hashlib
import struct
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "assets/app-icon"
MASTER = ICONS / "figcraft-1024.png"

PALETTE_FIELD = (0x02, 0xda, 0x96)  # FigCraft green, the committed field
NIGHTLY_FIELD = (0x01, 0x96, 0x68)  # the same field, deeper: one third of the way to black
MASTER_SIZE = 1024
# Windows frames: 32bpp DIBs up to 128 px (what every resource compiler and Explorer accepts) and
# one PNG frame for 256. macOS: PNG payloads per slot, including the @2x slots iconutil expects.
ICO_DIB_SIZES = (16, 32, 48, 64, 128)
ICO_PNG_SIZES = (256,)
ICNS_SLOTS = ((b"icp4", 16), (b"icp5", 32), (b"ic11", 32), (b"icp6", 64), (b"ic12", 64), (b"ic07", 128), (b"ic08", 256), (b"ic13", 256), (b"ic09", 512), (b"ic14", 512), (b"ic10", 1024))
PNG_SIZES = (1024, 512, 256, 128, 64, 48, 32, 16)


def read_png(path):
    """The image's width, height and RGBA rows of an 8-bit PNG (no interlace)."""
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path} is not a PNG")
    offset, header, idat = 8, None, bytearray()
    while offset < len(data):
        (length,) = struct.unpack(">I", data[offset : offset + 4])
        kind = data[offset + 4 : offset + 8]
        body = data[offset + 8 : offset + 8 + length]
        offset += 12 + length
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat += body
        elif kind == b"IEND":
            break
    if header is None:
        raise ValueError(f"{path} has no IHDR")
    width, height, depth, colour, compression, filtering, interlace = header
    if depth != 8 or compression != 0 or filtering != 0 or interlace != 0 or colour not in (2, 6):
        raise ValueError(f"{path}: expected a non-interlaced 8-bit RGB or RGBA PNG")
    channels = 4 if colour == 6 else 3
    stride = width * channels
    raw = zlib.decompress(bytes(idat))
    if len(raw) != (stride + 1) * height:
        raise ValueError(f"{path}: unexpected pixel data length")
    rows, previous = [], bytearray(stride)
    for y in range(height):
        start = y * (stride + 1)
        filter_type = raw[start]
        line = bytearray(raw[start + 1 : start + 1 + stride])
        unfilter(filter_type, line, previous, channels)
        rows.append(bytes(line))
        previous = line
    if channels == 3:
        rows = [bytes(component for x in range(width) for component in (*row[3 * x : 3 * x + 3], 255)) for row in rows]
    return width, height, rows


def unfilter(filter_type, line, previous, bpp):
    if filter_type == 0:
        return
    if filter_type == 1:
        for i in range(bpp, len(line)):
            line[i] = (line[i] + line[i - bpp]) & 0xFF
    elif filter_type == 2:
        for i in range(len(line)):
            line[i] = (line[i] + previous[i]) & 0xFF
    elif filter_type == 3:
        for i in range(len(line)):
            left = line[i - bpp] if i >= bpp else 0
            line[i] = (line[i] + ((left + previous[i]) >> 1)) & 0xFF
    elif filter_type == 4:
        for i in range(len(line)):
            left = line[i - bpp] if i >= bpp else 0
            up = previous[i]
            upleft = previous[i - bpp] if i >= bpp else 0
            estimate = left + up - upleft
            pa, pb, pc = abs(estimate - left), abs(estimate - up), abs(estimate - upleft)
            predictor = left if pa <= pb and pa <= pc else (up if pb <= pc else upleft)
            line[i] = (line[i] + predictor) & 0xFF
    else:
        raise ValueError(f"unknown PNG filter type {filter_type}")


def deepen(row, width):
    """One row of the master's RGBA with only the field colour moved."""
    delta = (NIGHTLY_FIELD[0] - PALETTE_FIELD[0], NIGHTLY_FIELD[1] - PALETTE_FIELD[1], NIGHTLY_FIELD[2] - PALETTE_FIELD[2])
    between = (PALETTE_FIELD[0] - PALETTE_FIELD[1], PALETTE_FIELD[1] - PALETTE_FIELD[2])
    denominator = between[0] * between[0] + between[1] * between[1]
    out = bytearray(row)
    for x in range(width):
        i = x * 4
        alpha = row[i + 3]
        if alpha == 0:
            continue
        red, green, blue = row[i], row[i + 1], row[i + 2]
        # ink and paper are neutral, so a pixel's saturation is exactly its field contribution
        weight = ((red - green) * between[0] + (green - blue) * between[1]) / denominator
        if weight <= 0:
            continue
        weight = min(weight, 1.0)
        for channel in range(3):
            out[i + channel] = min(255, max(0, round(row[i + channel] + weight * delta[channel])))
    return bytes(out)


def taps(width, size):
    """For each output index: the source indices it covers and their fractional weights."""
    scale = width / size
    out = []
    for index in range(size):
        low, high = index * scale, (index + 1) * scale
        first, last = int(low), min(width, int(high) + (1 if high > int(high) else 0))
        weights = [(source, min(high, source + 1) - max(low, source)) for source in range(first, last)]
        total = sum(weight for _, weight in weights)
        out.append([(source, weight / total) for source, weight in weights if weight > 0])
    return out


def average(rows, width, size):
    """The area average of the `width`-square image into `size` square rows, in premultiplied alpha.

    Any ratio works (a 48 px frame is 21.33 source pixels wide), so every size is a true average of
    the master rather than a crop or a nearest sample.
    """
    if width == size:
        return rows
    columns = taps(width, size)
    # Horizontal pass: premultiplied RGBA per output column.
    horizontal = []
    for row in rows:
        line = []
        for weights in columns:
            red = green = blue = alpha = 0.0
            for source, weight in weights:
                i = source * 4
                a = row[i + 3]
                red += row[i] * a * weight
                green += row[i + 1] * a * weight
                blue += row[i + 2] * a * weight
                alpha += a * weight
            line.append((red, green, blue, alpha))
        horizontal.append(line)
    out = []
    for weights in taps(width, size):
        row = bytearray(size * 4)
        for x in range(size):
            red = green = blue = alpha = 0.0
            for source, weight in weights:
                r, g, b, a = horizontal[source][x]
                red += r * weight
                green += g * weight
                blue += b * weight
                alpha += a * weight
            i = x * 4
            if alpha > 0:
                row[i] = min(255, round(red / alpha))
                row[i + 1] = min(255, round(green / alpha))
                row[i + 2] = min(255, round(blue / alpha))
            row[i + 3] = min(255, round(alpha))
        out.append(bytes(row))
    return out


def dib(rows, size):
    """A 32bpp bottom-up DIB frame with its 1bpp AND mask, as Explorer and rc.exe expect."""
    header = struct.pack("<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, size * size * 4, 0, 0, 0, 0)
    pixels = bytearray()
    for y in range(size - 1, -1, -1):
        line = rows[y]
        for x in range(size):
            i = x * 4
            pixels += bytes((line[i + 2], line[i + 1], line[i], line[i + 3]))
    mask_row = ((size + 31) // 32) * 4
    mask = bytearray(mask_row * size)
    for y in range(size):
        line = rows[y]
        target = (size - 1 - y) * mask_row
        for x in range(size):
            if line[x * 4 + 3] == 0:
                mask[target + (x // 8)] |= 0x80 >> (x % 8)
    return header + bytes(pixels) + bytes(mask)


def ico(frames):
    count = len(frames)
    header = struct.pack("<HHH", 0, 1, count)
    offset = 6 + 16 * count
    entries, blobs = bytearray(), bytearray()
    for size, blob in frames:
        dimension = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", dimension, dimension, 0, 0, 1, 32, len(blob), offset)
        blobs += blob
        offset += len(blob)
    return header + bytes(entries) + bytes(blobs)


def icns(payloads):
    body = bytearray()
    for kind, size in ICNS_SLOTS:
        blob = payloads[size]
        body += kind + struct.pack(">I", len(blob) + 8) + blob
    return b"icns" + struct.pack(">I", len(body) + 8) + bytes(body)


def build():
    width, height, rows = read_png(MASTER)
    if (width, height) != (MASTER_SIZE, MASTER_SIZE):
        raise ValueError(f"{MASTER} must be {MASTER_SIZE}x{MASTER_SIZE}, not {width}x{height}")
    deep = [deepen(row, width) for row in rows]
    rendered = {size: average(deep, width, size) for size in PNG_SIZES}
    files = {}
    for size in PNG_SIZES:
        files[file_name(size)] = png_bytes(rendered[size], size)
    files["figcraft-nightly.ico"] = ico([(size, dib(rendered[size], size)) for size in ICO_DIB_SIZES] + [(size, files[file_name(size)]) for size in ICO_PNG_SIZES])
    files["figcraft-nightly.icns"] = icns({size: files[file_name(size)] for _, size in ICNS_SLOTS})
    return files


def file_name(size):
    """The committed file a frame of `size` px lives in (the 512 one is the plain nightly PNG)."""
    return "figcraft-nightly.png" if size == 512 else f"figcraft-nightly-{size}.png"


def png_bytes(rows, size):
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)

    raw = b"".join(b"\x00" + row for row in rows)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="report differences instead of writing")
    args = parser.parse_args()
    files = build()
    stale = []
    for name, data in sorted(files.items()):
        path = ICONS / name
        if args.check:
            if not path.exists() or path.read_bytes() != data:
                stale.append(name)
            continue
        path.write_bytes(data)
        print(f"{name}: {hashlib.sha256(data).hexdigest()}")
    if args.check:
        if stale:
            print("out of date: " + ", ".join(stale), file=sys.stderr)
            return 1
        print("the committed nightly icons match this script")
    return 0


if __name__ == "__main__":
    sys.exit(main())
