#!/usr/bin/env python3
"""The Nightly icons: the generator's output, the containers Windows and macOS read, and the
provenance record. No image tool, app build or signing is involved."""
import hashlib
import importlib.util
import json
import struct
import unittest
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
ICONS = ROOT / "assets/app-icon"
SPEC = importlib.util.spec_from_file_location("figcraft_nightly_icons", HERE / "nightly-icons.py")
ICON = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ICON)
PROVENANCE = json.loads((ICONS / "nightly-provenance.json").read_text())


def png_size(data):
    if data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR":
        raise ValueError("not a PNG")
    return struct.unpack(">II", data[16:24])


class Containers(unittest.TestCase):
    def test_ico_holds_every_frame_at_the_size_it_names(self):
        data = (ICONS / "figcraft-nightly.ico").read_bytes()
        reserved, kind, count = struct.unpack("<HHH", data[:6])
        self.assertEqual((reserved, kind), (0, 1))
        self.assertEqual(count, len(ICON.ICO_DIB_SIZES) + len(ICON.ICO_PNG_SIZES))
        sizes = []
        for index in range(count):
            width, height, colours, _, planes, bits, length, offset = struct.unpack("<BBBBHHII", data[6 + 16 * index : 22 + 16 * index])
            self.assertEqual((colours, planes, bits), (0, 1, 32))
            self.assertEqual(width, height)
            sizes.append(256 if width == 0 else width)
            blob = data[offset : offset + length]
            self.assertEqual(len(blob), length)
            if width == 0:
                self.assertEqual(png_size(blob), (256, 256), "the 256 px frame is a PNG")
            else:
                header = struct.unpack("<IiiHHIIiiII", blob[:40])
                self.assertEqual(header[0], 40, "a BITMAPINFOHEADER")
                self.assertEqual((header[1], header[2]), (width, width * 2), "the doubled height covers the AND mask")
                self.assertEqual(header[4], 32)
                self.assertEqual(len(blob), 40 + width * width * 4 + ((width + 31) // 32) * 4 * width)
        self.assertEqual(sizes, list(ICON.ICO_DIB_SIZES) + list(ICON.ICO_PNG_SIZES))

    def test_icns_holds_every_png_slot_at_its_size(self):
        data = (ICONS / "figcraft-nightly.icns").read_bytes()
        self.assertEqual(data[:4], b"icns")
        self.assertEqual(struct.unpack(">I", data[4:8])[0], len(data))
        offset = 8
        found = {}
        while offset < len(data):
            kind = data[offset : offset + 4]
            (length,) = struct.unpack(">I", data[offset + 4 : offset + 8])
            blob = data[offset + 8 : offset + length]
            self.assertEqual(png_size(blob), (ICON_SIZE[kind],) * 2, kind)
            self.assertNotIn(kind, found, "a duplicated slot")
            found[kind] = offset
            offset += length
        self.assertEqual(set(found), {kind for kind, _ in ICON.ICNS_SLOTS})

    def test_png_frames_are_well_formed_and_distinct_sizes(self):
        for size in ICON.PNG_SIZES:
            path = ICONS / ICON.file_name(size)
            data = path.read_bytes()
            self.assertEqual(png_size(data), (size, size), path.name)
            # Decompressing the frames proves the CRCs and the zlib stream are intact.
            offset, idat = 8, b""
            while offset < len(data):
                (length,) = struct.unpack(">I", data[offset : offset + 4])
                kind = data[offset + 4 : offset + 8]
                body = data[offset + 8 : offset + 8 + length]
                self.assertEqual(zlib.crc32(kind + body) & 0xFFFFFFFF, struct.unpack(">I", data[offset + 8 + length : offset + 12 + length])[0])
                if kind == b"IDAT":
                    idat += body
                offset += 12 + length
            self.assertEqual(len(zlib.decompress(idat)), (size * 4 + 1) * size, path.name)

    def test_the_nightly_field_is_deeper_and_the_palette_is_the_committed_one(self):
        self.assertEqual(ICON.PALETTE_FIELD, (0x02, 0xDA, 0x96), "the stable icon's field colour")
        self.assertEqual(ICON.NIGHTLY_FIELD, tuple(bytes.fromhex(PROVENANCE["method"]["nightly_background"].lstrip("#"))))
        stable = json.loads((ICONS / "nightly-provenance.json").read_text())["method"]["original_palette"]["background"]
        self.assertEqual(stable, "#02da96")
        self.assertLess(sum(ICON.NIGHTLY_FIELD), sum(ICON.PALETTE_FIELD), "a deeper field, not a brighter one")

    def test_the_committed_files_are_what_the_generator_makes(self):
        for name, data in ICON.build().items():
            committed = (ICONS / name).read_bytes()
            self.assertEqual(committed, data, name)


class Provenance(unittest.TestCase):
    def test_every_recorded_input_and_output_matches_its_sha256(self):
        for entry in PROVENANCE["inputs"]:
            path = ROOT / entry["path"]
            self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), entry["sha256"], entry["path"])
        for name, digest in PROVENANCE["outputs"].items():
            path = ROOT / name
            self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), digest, name)
        generated = {f"assets/app-icon/{name}" for name in ICON.build()}
        self.assertEqual(set(PROVENANCE["outputs"]), generated, "every generated file is recorded")

    def test_the_record_names_the_generator_that_made_the_bytes(self):
        generator = HERE / "nightly-icons.py"
        self.assertEqual(hashlib.sha256(generator.read_bytes()).hexdigest(), PROVENANCE["method"]["generator_sha256"])
        self.assertEqual(PROVENANCE["ico_frame_sizes"], list(ICON.ICO_DIB_SIZES) + list(ICON.ICO_PNG_SIZES))
        self.assertEqual(PROVENANCE["icns_png_slots"], {kind.decode(): size for kind, size in ICON.ICNS_SLOTS})

    def test_the_slots_match_the_frames_they_name(self):
        self.assertEqual(sorted(PROVENANCE["ico_frame_sizes"]), [16, 32, 48, 64, 128, 256])


# The sizes the committed .icns slots carry, as the container test reads them.
ICON_SIZE = {kind: size for kind, size in ICON.ICNS_SLOTS}


if __name__ == "__main__":
    unittest.main()
