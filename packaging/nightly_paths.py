"""POSIX packaging I/O rooted at a caller-owned private working directory.

The build launcher chooses cwd independently of file arguments (Linux uses its
mktemp directory). This is not a sandbox against root or a malicious same-UID
caller, and inode-check/unlink cleanup is not an atomic replacement-race primitive.
"""
import contextlib
import os
from pathlib import Path
import stat


def _component(name):
    # A descriptor-relative open must name exactly one entry in its directory.
    if not name or name in (".", "..") or os.path.basename(name) != name:
        raise ValueError("build path components must be single names")
    return os.path.basename(name)


class OwnedBuildRoot:
    def __enter__(self):
        self.directory = Path.cwd()
        self.fd = os.open(".", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        info = os.fstat(self.fd)
        if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) & 0o077:
            os.close(self.fd)
            raise ValueError("packaging cwd must be an owned private build directory")
        return self

    def __exit__(self, *_error):
        os.close(self.fd)

    def _relative(self, path):
        path = Path(path)
        try:
            relative = path.relative_to(self.directory) if path.is_absolute() else path
        except ValueError as error:
            raise ValueError("path is outside the owned build directory") from error
        if not relative.parts or ".." in relative.parts:
            raise ValueError("empty or parent-traversing build path")
        # Check an independently selected root, never the supplied filename's parent.
        try:
            parent = (self.directory / relative).parent.resolve(strict=True)
        except OSError as error:
            raise ValueError("build path parent must exist") from error
        if not parent.is_relative_to(self.directory):
            raise ValueError("resolved path escapes the owned build directory")
        return relative

    def _checked_parts(self, path):
        relative = self._relative(path)
        root = os.path.realpath(self.directory)
        lexical = os.path.join(root, *relative.parts)
        target = os.path.realpath(lexical)
        if target == root or os.path.commonpath([root, target]) != root:
            raise ValueError("resolved path escapes the owned build directory")
        if target != lexical:
            raise ValueError("build path must not pass through symlinks or aliases")
        return [_component(part) for part in os.path.relpath(target, root).split(os.sep)]

    def _parent(self, path):
        parts = self._checked_parts(path)
        current = os.dup(self.fd)
        try:
            for part in parts[:-1]:
                next_fd = os.open(_component(part), os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=current)
                os.close(current)
                current = next_fd
            return current, parts[-1]
        except OSError as error:
            os.close(current)
            raise ValueError("build path parents must be non-symlink directories") from error

    @contextlib.contextmanager
    def reader(self, path, limit):
        parent, name = self._parent(path)
        try:
            try:
                descriptor = os.open(_component(name), os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
            except OSError as error:
                raise ValueError("build input must be a regular file without aliases") from error
        finally:
            os.close(parent)
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode) or info.st_size > limit:
            os.close(descriptor)
            raise ValueError("build input must be a regular file within its read bound")
        with os.fdopen(descriptor, "rb") as stream:
            yield stream

    def read_bytes(self, path, limit):
        with self.reader(path, limit) as stream:
            data = stream.read(limit + 1)
        if len(data) > limit:
            raise ValueError("build input grew beyond its read bound")
        return data

    def children(self, path, limit):
        relative = self._relative(path)
        parent, name = self._parent(path)
        try:
            try:
                descriptor = os.open(_component(name), os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
            except OSError as error:
                raise ValueError("build stage must be a non-symlink directory") from error
        finally:
            os.close(parent)
        try:
            entries = []
            with os.scandir(descriptor) as iterator:
                for entry in iterator:
                    if len(entries) >= limit:
                        raise ValueError("too many build stage members")
                    entries.append(self.directory / relative / entry.name)
            return entries
        finally:
            os.close(descriptor)

    @contextlib.contextmanager
    def create(self, path):
        parent, name = self._parent(path)
        name = _component(name)
        try:
            descriptor = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o666, dir_fd=parent)
            info = os.fstat(descriptor)
            identity = (info.st_dev, info.st_ino)
            try:
                with os.fdopen(descriptor, "wb") as stream:
                    yield stream
            except BaseException:
                try:
                    current = os.stat(name, dir_fd=parent, follow_symlinks=False)
                    if (current.st_dev, current.st_ino) == identity:
                        os.unlink(_component(name), dir_fd=parent)
                except FileNotFoundError:
                    pass
                raise
        finally:
            os.close(parent)
