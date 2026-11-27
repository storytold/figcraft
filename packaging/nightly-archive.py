#!/usr/bin/env python3
"""Flat Linux archive policy, invoked from its private owned build cwd.

The stage and archive must both remain under that independently chosen root.
No native build, platform signing, or malicious-same-UID sandbox is implied.
"""
import grp
import gzip
import os
import pathlib
import pwd
import re
import stat
import sys
import tarfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from nightly_paths import OwnedBuildRoot

APP_DOCS = {"README.md", "LICENSE", "LICENSE-MIT", "LICENSE-APACHE", "COPYRIGHT", "NOTICE", "ATTRIBUTION.md"}
MAX_MEMBER_BYTES = 512 * 1024 * 1024
MAX_TOTAL_BYTES = 1024 * 1024 * 1024
MAX_MEMBERS = 128


def notice(name):
    return name in APP_DOCS or re.fullmatch(r"OFL-[A-Za-z0-9_.-]+\.txt", name) is not None


def checked_entries(root, stage):
    entries = sorted(root.children(stage, MAX_MEMBERS), key=lambda item: item.name)
    folded = set()
    total = 0
    for entry in entries:
        name = entry.name
        if name not in {"figcraft", "figcraft-bin", "figcraft-nightly.png"} and not notice(name):
            raise ValueError(f"unexpected nightly member: {name}")
        if len(name.encode("utf-8")) > 100:
            raise ValueError(f"nightly member must be a short regular file: {name}")
        with root.reader(entry, MAX_MEMBER_BYTES) as stream:
            total += os.fstat(stream.fileno()).st_size
        if total > MAX_TOTAL_BYTES:
            raise ValueError("nightly stage exceeds its read bound")
        if name.casefold() in folded:
            raise ValueError(f"case-colliding nightly member: {name}")
        folded.add(name.casefold())
    # The standard nightly layout: figcraft is the launcher that starts the GUI on the Launcher's
    # nightly-v1 profile (packaging/linux/nightly-launch.sh), and figcraft-bin is that GUI, so an
    # unpacked archive is isolated too.
    required = {"figcraft", "figcraft-bin", "figcraft-nightly.png", "README.md", "LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "ATTRIBUTION.md"}
    if not required.issubset({item.name for item in entries}):
        raise ValueError("required GUI/icon/notices missing")
    return entries


def member_info(stage_root, name, incoming):
    """Header for one stage file, built from the file this run opened rather than from the archive.

    Members are flat names that resolve directly inside the stage they came from.
    """
    if os.path.isabs(name) or os.path.basename(name) != name or name in ("", ".", ".."):
        raise ValueError(f"nightly member name must be a flat file name: {name}")
    destination = os.path.realpath(os.path.join(stage_root, name))
    if not destination.startswith(stage_root + os.sep) or os.path.dirname(destination) != stage_root:
        raise ValueError(f"nightly member resolves outside its stage: {name}")
    status = os.fstat(incoming.fileno())
    if not stat.S_ISREG(status.st_mode):
        raise ValueError(f"nightly member must be a regular file: {name}")
    # The same fields TarFile.gettarinfo records for a regular file.
    # The header keeps the validated stage name, not the resolved one: a stage symlink must
    # never rename a member or leave a required name out of the archive.
    info = tarfile.TarInfo(name)
    info.type = tarfile.REGTYPE
    info.mode = stat.S_IMODE(status.st_mode)
    info.uid, info.gid = status.st_uid, status.st_gid
    info.size = status.st_size
    info.mtime = status.st_mtime
    try:
        info.uname = pwd.getpwuid(status.st_uid).pw_name
    except KeyError:
        pass
    try:
        info.gname = grp.getgrgid(status.st_gid).gr_name
    except KeyError:
        pass
    return info


def verify_headers(root, output):
    # Bounded ordinary headers, not a transparent PAX/GNU-normalizing reader.
    total = 0
    count = 0
    with root.reader(output, MAX_TOTAL_BYTES) as incoming, gzip.GzipFile(fileobj=incoming) as stream:
        while True:
            header = stream.read(512)
            if header == b"\0" * 512:
                break
            if len(header) != 512 or header[156:157] not in {b"0", b"\0"}:
                raise ValueError("non-ordinary TAR record was emitted")
            size = int(header[124:136].rstrip(b"\0 ").lstrip(b" ") or b"0", 8)
            count += 1
            total += size
            if count > MAX_MEMBERS or size > MAX_MEMBER_BYTES or total > MAX_TOTAL_BYTES:
                raise ValueError("archive exceeds its bounded member budget")
            remaining = ((size + 511) // 512) * 512
            while remaining:
                chunk = stream.read(min(remaining, 1024 * 1024))
                if not chunk:
                    raise ValueError("truncated TAR member")
                remaining -= len(chunk)


def linux_archive(stage, output):
    with OwnedBuildRoot() as root:
        entries = checked_entries(root, stage)
        stage_root = os.path.realpath(os.path.join(root.directory, stage))
        total = 0
        with root.create(output) as outgoing:
            with tarfile.open(fileobj=outgoing, mode="w:gz", format=tarfile.USTAR_FORMAT) as archive:
                for entry in entries:
                    with root.reader(entry, MAX_MEMBER_BYTES) as incoming:
                        info = member_info(stage_root, entry.name, incoming)
                        total += info.size
                        if info.size > MAX_MEMBER_BYTES or total > MAX_TOTAL_BYTES:
                            raise ValueError("nightly stage grew beyond its read bound")
                        archive.addfile(info, incoming)
            outgoing.flush()
            os.fsync(outgoing.fileno())
            verify_headers(root, output)


if __name__ == "__main__":
    if len(sys.argv) != 4 or sys.argv[1] != "linux":
        raise SystemExit("usage: nightly-archive.py linux STAGE OUTPUT (from owned private build cwd)")
    linux_archive(pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3]))
