#!/usr/bin/env python3
"""Nightly metadata: run from a private owned build cwd with a staged template.

All file arguments must stay beneath that cwd; this is not native/signing acceptance.
"""
import argparse
import datetime
import os
from pathlib import Path
import plistlib
import re
import sys

# The shared helper location is fixed by this script, not by CLI file arguments.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from nightly_paths import OwnedBuildRoot

MAX_TEMPLATE_BYTES = 256 * 1024

IDENTIFIER = "ai.storyteller.figcraft.nightly"
# This app's template keys: CFBundleExecutable, CFBundleIconFile and the version key are its own.
EXECUTABLE = "FigCraft"
ICON = "figcraft-nightly"
VERSION_KEY = "FigCraftVersion"
COMMIT_KEY = "FigCraftBuildCommit"
ASSOCIATIONS = ("CFBundleDocumentTypes", "CFBundleURLTypes", "UTExportedTypeDeclarations", "UTImportedTypeDeclarations")
VERSION = re.compile(r"([0-9]+\.[0-9]+\.[0-9]+)-nightly\.([0-9]{8})\.[0-9]+\.[0-9]+\.g([0-9a-f]{12})\Z")


def render(template: bytes, version: str, commit: str) -> bytes:
    match = VERSION.fullmatch(version)
    if not match or not re.fullmatch(r"[0-9a-f]{40}", commit) or match[3] != commit[:12]:
        raise ValueError("exact prepared Nightly version and matching source commit are required")
    datetime.datetime.strptime(match[2], "%Y%m%d")
    data = plistlib.loads(template)
    if not isinstance(data, dict) or data.get("CFBundlePackageType") != "APPL":
        raise ValueError("expected the app-owned application plist template")
    for key in ASSOCIATIONS:
        data.pop(key, None)
    data.update(
        CFBundleDisplayName="FigCraft Nightly",
        CFBundleName="FigCraft Nightly",
        CFBundleIdentifier=IDENTIFIER,
        CFBundleExecutable=EXECUTABLE,
        CFBundleIconFile=ICON,
        CFBundleShortVersionString=match[1],
        CFBundleVersion=match[1],
    )
    data[VERSION_KEY] = version
    data[COMMIT_KEY] = commit
    return plistlib.dumps(data, fmt=plistlib.FMT_XML, sort_keys=False)


def write_plist(template: Path, destination: Path, version: str, commit: str) -> None:
    with OwnedBuildRoot() as root:
        content = render(root.read_bytes(template, MAX_TEMPLATE_BYTES), version, commit)
        with root.create(destination) as stream:
            stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--template", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--source-commit", required=True)
    args = parser.parse_args()
    write_plist(args.template, args.output, args.version, args.source_commit)


if __name__ == "__main__":
    main()
