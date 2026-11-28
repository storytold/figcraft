#!/usr/bin/env python3
"""The macOS Nightly signing and verification scripts, run on any POSIX host against generated
fixtures and stand-in Apple tools (codesign, notarytool, stapler, spctl, security, ditto, plutil,
lipo) that log their arguments. This proves the scripts' order, refusals and cleanup; it is not a
real signature, notarization or Mac run."""
import base64
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import unittest
import zipfile

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("fig_mac_nightly_plist", HERE / "nightly_plist.py")
PLIST = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PLIST)
COMMIT = "b" * 40
VERSION = "0.18.1-nightly.20261124.4567.1.g" + COMMIT[:12]
ASSET = f"figcraft-{VERSION}-macos-aarch64-nightly.zip"
APP = "FigCraft Nightly.app"
TEAM = "ABCDE12345"
IDENTITY = "0123456789ABCDEF0123456789ABCDEF01234567"
SECRETS = {
    "APPLE_CERTIFICATE": base64.b64encode(b"generated fixture, not a certificate").decode(),
    "APPLE_CERTIFICATE_PASSWORD": "generated-fixture",
    "KEYCHAIN_PASSWORD": "generated-fixture",
    "APPLE_ID": "fixture@example.invalid",
    "APPLE_PASSWORD": "generated-fixture",
    "APPLE_TEAM_ID": TEAM,
}

# One stand-in for every Apple tool, dispatched on its own name. Each logs its argv as a JSON line.
SHIM = r"""
import json, os, plistlib, sys, zipfile
from pathlib import Path
name = Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ["SHIM_LOG"], "a") as log:
    log.write(json.dumps([name] + args) + "\n")
team = os.environ.get("SHIM_TEAM", "")
if name == "uname":
    print({"-s": "Darwin", "-m": "arm64"}.get(args[0], "Darwin"))
elif name == "lipo":
    print("arm64")
elif name == "ditto":
    paths = [a for a in args if not a.startswith("-")]
    src, dst = paths
    if "-x" in args:
        with zipfile.ZipFile(src) as z:
            for info in z.infolist():
                target = Path(dst) / info.filename
                if info.is_dir():
                    target.mkdir(parents=True, exist_ok=True)
                    continue
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(z.read(info))
                mode = (info.external_attr >> 16) & 0o777
                if mode:
                    target.chmod(mode)
    else:
        root = Path(src)
        with zipfile.ZipFile(dst, "x") as z:
            for path in sorted([root] + list(root.rglob("*"))):
                arc = (Path(root.name) / path.relative_to(root)).as_posix()
                if path.is_dir():
                    z.writestr(zipfile.ZipInfo(arc + "/"), b"")
                else:
                    info = zipfile.ZipInfo(arc)
                    info.external_attr = (path.stat().st_mode & 0xFFFF) << 16
                    z.writestr(info, path.read_bytes())
elif name == "plutil":
    if args[0] == "-lint":
        sys.exit(0)
    key, fmt, source = args[1], args[2], args[-1]
    data = sys.stdin.read() if source == "-" else Path(source).read_bytes()
    value = json.loads(data) if source == "-" else plistlib.loads(data)
    if key not in value:
        sys.exit(1)
    print(value[key] if fmt == "raw" else "<present/>")
elif name == "security":
    verb = args[0]
    if verb == "create-keychain":
        Path(args[-1]).write_bytes(b"keychain")
    elif verb == "find-identity":
        print('  1) %s "Developer ID Application: Generated Fixture (%s)"' % (os.environ["SHIM_IDENTITY"], team))
        print("     1 valid identities found")
    elif verb == "delete-keychain":
        Path(args[-1]).unlink()
elif name == "codesign":
    target = Path(args[-1])
    if "--sign" in args and target.suffix == ".app":
        (target / "Contents/_CodeSignature").mkdir(exist_ok=True)
        (target / "Contents/_CodeSignature/CodeResources").write_bytes(b"sealed")
    if "-d" in args:
        sys.stderr.write("Executable=%s\nIdentifier=ai.storyteller.figcraft.nightly\n"
                         "CodeDirectory v=20500 size=1 flags=0x10000(runtime) hashes=1+7 location=embedded\n"
                         "Authority=Developer ID Application: Generated Fixture (%s)\n"
                         "Timestamp=24 Nov 2026 at 17:00:00\nTeamIdentifier=%s\n" % (target, team, team))
elif name == "xcrun":
    if args[:2] == ["notarytool", "submit"]:
        print(json.dumps({"id": "generated-id", "status": os.environ.get("SHIM_NOTARY", "Accepted")}))
    elif args[:2] == ["stapler", "staple"]:
        (Path(args[2]) / "Contents/CodeResources").write_bytes(b"ticket")
    elif args[:2] == ["stapler", "validate"]:
        sys.exit(0 if (Path(args[2]) / "Contents/CodeResources").is_file() else 1)
elif name == "spctl":
    sys.stderr.write("%s: accepted\nsource=Notarized Developer ID\n" % args[-1])
elif name == "cargo":
    sys.exit(1)
# sleep: the retry waits pass at once.
"""

TOOLS = ("uname", "lipo", "ditto", "plutil", "security", "codesign", "xcrun", "spctl", "cargo", "sleep")


class Fixture:
    def __init__(self, test):
        self.directory = tempfile.TemporaryDirectory()
        test.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        shim = self.bin / "shim.py"
        shim.write_text(SHIM)
        for tool in TOOLS:
            path = self.bin / tool
            path.write_text(f"#!{sys.executable}\nimport sys\nsys.argv[0] = {str(path)!r}\nexec(compile(open({str(shim)!r}).read(), 'shim', 'exec'), {{'__name__': '__main__'}})\n")
            path.chmod(0o755)
        self.log = self.root / "tools.log"
        self.ran = self.root / "payload-ran"
        self.temp = self.root / "runner-temp"
        self.temp.mkdir()
        self.dist = self.root / "unsigned"
        self.dist.mkdir()
        self.signed = self.root / "signed"
        self.signed.mkdir()
        self.input = self.dist / ASSET
        self.output = self.signed / ASSET
        # Every script runs from a directory with modules a Python helper must not import (-I).
        self.cwd = self.root / "cwd"
        self.cwd.mkdir()
        self.hijacked = self.root / "hijacked"
        for module in ("shutil", "zipfile", "stat"):
            (self.cwd / f"{module}.py").write_text(f"open({str(self.hijacked)!r}, 'a').write({module!r})\n")

    def add_entry(self, info, data=b""):
        """Append one raw entry to the input archive."""
        with zipfile.ZipFile(self.input, "a") as z:
            z.writestr(info, data)

    def bundle(self, version=VERSION, extra=None, gui=None):
        """The archive nightly.sh makes: the real template through the real plist helper."""
        stage = self.root / "stage"
        shutil.rmtree(stage, ignore_errors=True)
        app = stage / APP
        (app / "Contents/MacOS").mkdir(parents=True)
        (app / "Contents/Resources").mkdir()
        exe = app / "Contents/MacOS/FigCraft"
        exe.write_text(gui or f"#!/bin/sh\ntouch {self.ran}\necho 'FigCraft {version} (nightly-v1)'\n")
        exe.chmod(0o755)
        (app / "Contents/Resources/figcraft-nightly.icns").write_bytes(b"generated icon")
        rendered = PLIST.render((HERE / "Info.plist.in").read_bytes(), version, COMMIT)
        (app / "Contents/Info.plist").write_bytes(rendered)
        for relative, data in (extra or {}).items():
            (app / relative).write_bytes(data)
            (app / relative).chmod(0o755)
        if self.input.exists():
            self.input.unlink()
        with zipfile.ZipFile(self.input, "x") as z:
            for path in sorted([app] + list(app.rglob("*"))):
                arc = (Path(APP) / path.relative_to(app)).as_posix()
                info = zipfile.ZipInfo(arc + ("/" if path.is_dir() else ""))
                info.external_attr = (path.stat().st_mode & 0xFFFF) << 16
                z.writestr(info, b"" if path.is_dir() else path.read_bytes())
        return self.input

    def env(self, **changes):
        env = {"PATH": f"{self.bin}:{os.environ['PATH']}", "HOME": str(self.root), "SHIM_LOG": str(self.log),
               "SHIM_TEAM": TEAM, "SHIM_IDENTITY": IDENTITY, "RUNNER_TEMP": str(self.temp), **SECRETS}
        env.update(changes)
        return {key: value for key, value in env.items() if value is not None}

    def run(self, script, *args, **env):
        if self.log.exists():
            self.log.unlink()
        return subprocess.run(["bash", str(HERE / script), *map(str, args)], env=self.env(**env), cwd=self.cwd,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=180)

    def sign(self, **env):
        return self.run("nightly-sign.sh", "--arch", "aarch64", "--input", self.input, "--output", self.output, **env)

    def calls(self):
        if not self.log.exists():
            return []
        return [json.loads(line) for line in self.log.read_text().splitlines()]


def lib(function, *args):
    """Run one nightly-lib.sh function; returns (exit status, stdout)."""
    script = f'. {str(HERE / "nightly-lib.sh")!r}; {function} "$@"'
    result = subprocess.run(["bash", "-c", script, "lib", *args], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    return result.returncode, result.stdout.strip()


DETAILS = (f"Identifier=ai.storyteller.figcraft.nightly\nCodeDirectory v=20500 size=1 flags=0x10000(runtime) hashes=1\n"
           f"Authority=Developer ID Application: Generated Fixture ({TEAM})\nAuthority=Developer ID Certification Authority\n"
           f"Timestamp=24 Nov 2026\nTeamIdentifier={TEAM}\n")


class Checks(unittest.TestCase):
    def test_the_version_comes_only_from_a_prepared_nightly_asset_name(self):
        self.assertEqual(lib("mac_nightly_version_from_asset", ASSET), (0, VERSION))
        for name in (f"figcraft-0.18.1-macos-aarch64.zip", f"figcraft-{VERSION}-macos-universal-nightly.zip",
                     f"figcraft-{VERSION}-linux-x86_64-nightly.tar.gz", ASSET.replace(".g", ".x"), "../" + ASSET):
            with self.subTest(name=name):
                self.assertNotEqual(lib("mac_nightly_version_from_asset", name)[0], 0)

    def test_signature_needs_every_part_of_the_developer_id_signature(self):
        self.assertEqual(lib("mac_nightly_check_signature", DETAILS, TEAM)[0], 0)
        for broken in (DETAILS.replace("flags=0x10000(runtime)", "flags=0x0(none)"),
                       DETAILS.replace("Timestamp=24 Nov 2026\n", "Signed Time=24 Nov 2026\n"),
                       "Identifier=figcraft-1234\nSignature=adhoc\nTeamIdentifier=not set\n",
                       DETAILS.replace("ai.storyteller.figcraft.nightly", "ai.storyteller.figcraft"),
                       DETAILS.replace("ai.storyteller.figcraft.nightly", "aiXstorytellerXfigcraftXnightly")):
            with self.subTest(broken=broken):
                self.assertNotEqual(lib("mac_nightly_check_signature", broken, TEAM)[0], 0)
        self.assertNotEqual(lib("mac_nightly_check_signature", DETAILS, "ZZZZZ99999")[0], 0, "another team")

    def test_this_run_owns_only_a_keychain_named_for_the_app_inside_it(self):
        with tempfile.TemporaryDirectory() as work:
            # import-cert.sh puts the keychain in its own mktemp directory below the run directory.
            nested = Path(work) / "figcraft-signing.AbCdEf"
            nested.mkdir()
            ours = nested / "figcraft-signing.keychain-db"
            ours.write_bytes(b"keychain")
            self.assertEqual(lib("mac_nightly_keychain_is_ours", str(ours), work)[0], 0)
            direct = Path(work) / "figcraft-signing.keychain-db"
            direct.write_bytes(b"keychain")
            self.assertEqual(lib("mac_nightly_keychain_is_ours", str(direct), work)[0], 0)
            for label, path in (("another app's name", nested / "figcraft-ui-signing.keychain-db"),
                                ("outside the run", str(Path(work).parent / "figcraft-signing.keychain-db")),
                                ("a directory", str(nested)),
                                ("missing", str(nested / "missing.keychain-db")),
                                ("empty", "")):
                if label == "another app's name":
                    Path(path).write_bytes(b"keychain")
                with self.subTest(label=label):
                    self.assertNotEqual(lib("mac_nightly_keychain_is_ours", path, work)[0], 0)
            link = nested / "figcraft-signing.keychain-db"
            link.unlink()
            link.symlink_to(direct)
            self.assertNotEqual(lib("mac_nightly_keychain_is_ours", str(link), work)[0], 0, "never through a link")

    def test_gatekeeper_must_accept_the_bundle_as_notarized(self):
        self.assertEqual(lib("mac_nightly_check_assessment", "x.app: accepted\nsource=Notarized Developer ID\n")[0], 0)
        for text in ("x.app: accepted\nsource=Developer ID\n", "x.app: rejected\nsource=Unnotarized Developer ID\n"):
            with self.subTest(text=text):
                self.assertNotEqual(lib("mac_nightly_check_assessment", text)[0], 0)

    def test_the_imported_identity_must_be_a_developer_id_of_the_team(self):
        line = f'  1) {IDENTITY} "Developer ID Application: Generated Fixture ({TEAM})"'
        self.assertEqual(lib("mac_nightly_identity_is_team", line, IDENTITY, TEAM)[0], 0)
        self.assertNotEqual(lib("mac_nightly_identity_is_team", line, IDENTITY, "ZZZZZ99999")[0], 0)
        self.assertNotEqual(lib("mac_nightly_identity_is_team", line.replace("Developer ID Application", "Apple Development"), IDENTITY, TEAM)[0], 0)


class Sign(unittest.TestCase):
    def setUp(self):
        self.f = Fixture(self)
        self.f.bundle()
        self.unsigned = self.f.input.read_bytes()

    def assert_refused_untouched(self, result, signed=False):
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertFalse(self.f.output.exists())
        self.assertEqual(self.f.input.read_bytes(), self.unsigned)
        self.assertFalse(self.f.ran.exists(), "the payload must never run in the signing step")
        names = [call[0] for call in self.f.calls()]
        self.assertNotIn("cargo", names)
        if not signed:
            self.assertNotIn("codesign", names)
            self.assertNotIn("xcrun", names)
        self.assertEqual(list(self.f.temp.iterdir()), [], "the work directory and keychain are removed")

    def test_signs_notarizes_and_staples_without_running_the_payload(self):
        result = self.f.sign()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertFalse(self.f.hijacked.exists(), "helpers run isolated (python3 -I) beside the secrets")
        calls = self.f.calls()
        names = [call[0] for call in calls]
        self.assertNotIn("cargo", names)
        self.assertFalse(self.f.ran.exists(), "the payload must never run in the signing step")
        self.assertEqual(self.f.input.read_bytes(), self.unsigned)
        signs = [call for call in calls if call[0] == "codesign" and "--sign" in call]
        self.assertEqual([Path(call[-1]).name for call in signs], ["FigCraft", APP], "inside-out: executable, then bundle")
        for call in signs:
            for flag in ("--force", "--timestamp", "runtime", str(HERE / "entitlements.plist"), IDENTITY):
                self.assertIn(flag, call)
        # The keychain is the one import-cert.sh made in its own directory of this run.
        keychains = [call[call.index("--keychain") + 1] for call in signs]
        self.assertTrue(all(Path(path).name == "figcraft-signing.keychain-db" for path in keychains), keychains)
        self.assertTrue(all(Path(path).is_relative_to(self.f.temp) for path in keychains), keychains)
        order = [" ".join(call[:3]) for call in calls]
        submit = order.index("xcrun notarytool submit")
        staple = order.index("xcrun stapler staple")
        self.assertLess(max(i for i, call in enumerate(calls) if call[0] == "codesign" and "--sign" in call), submit)
        self.assertLess(submit, staple)
        final = [i for i, call in enumerate(calls) if call[0] == "ditto" and "-c" in call][-1]
        self.assertLess(staple, final, "the final archive carries the stapled ticket")
        self.assertIn("--noextattr", calls[final])
        self.assertIn(["security", "delete-keychain"], [call[:2] for call in calls])
        self.assertEqual(list(self.f.temp.iterdir()), [], "the work directory and keychain are removed")
        with zipfile.ZipFile(self.f.output) as z:
            names = set(z.namelist())
            self.assertIn(f"{APP}/Contents/CodeResources", names, "stapled ticket inside the bundle")
            self.assertIn(f"{APP}/Contents/_CodeSignature/CodeResources", names)
            self.assertTrue(all(name.startswith(APP) for name in names))
            mode = z.getinfo(f"{APP}/Contents/MacOS/FigCraft").external_attr >> 16
            self.assertTrue(mode & stat.S_IXUSR)

    def test_every_signing_and_notarization_secret_is_required(self):
        for name in SECRETS:
            with self.subTest(missing=name):
                self.assert_refused_untouched(self.f.sign(**{name: None}))
                self.assertEqual(self.f.calls(), [], "no tool runs before the configuration is complete")
        self.assert_refused_untouched(self.f.sign(NIGHTLY_APPLE_TEAM_ID="ZZZZZ99999"))
        self.assert_refused_untouched(self.f.sign(APPLE_TEAM_ID="not-a-team"))

    def test_refuses_another_team_s_identity(self):
        self.assert_refused_untouched(self.f.sign(SHIM_TEAM="ZZZZZ99999"))

    def test_never_replaces_an_output_or_renames_the_asset(self):
        self.f.output.write_bytes(b"generated existing artifact")
        result = self.f.sign()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.f.output.read_bytes(), b"generated existing artifact")
        self.f.output.unlink()
        renamed = self.f.run("nightly-sign.sh", "--arch", "aarch64", "--input", self.f.input, "--output", self.f.signed / "other.zip")
        self.assertNotEqual(renamed.returncode, 0)
        relative = self.f.run("nightly-sign.sh", "--arch", "aarch64", "--input", self.f.input.name, "--output", self.f.output)
        self.assertNotEqual(relative.returncode, 0)
        self.assertEqual(list(self.f.signed.iterdir()), [])

    def test_refuses_a_payload_that_is_not_the_exact_gui_only_nightly(self):
        cases = {
            "cli": dict(extra={"Contents/MacOS/figcraft-cli": b"generated cli"}),
            "version": dict(version=VERSION.replace("4567", "4568")),
        }
        for label, change in cases.items():
            with self.subTest(label=label):
                self.f.bundle(**change)
                self.unsigned = self.f.input.read_bytes()
                self.assert_refused_untouched(self.f.sign())

    def test_refuses_links_traversal_and_foreign_entries_before_unpacking(self):
        link = zipfile.ZipInfo(f"{APP}/Contents/Resources/link")
        link.external_attr = (stat.S_IFLNK | 0o777) << 16
        fifo = zipfile.ZipInfo(f"{APP}/Contents/Resources/fifo")
        fifo.external_attr = (stat.S_IFIFO | 0o644) << 16
        for label, info in (("link", link), ("special", fifo), ("traversal", zipfile.ZipInfo(f"{APP}/../outside")),
                            ("absolute", zipfile.ZipInfo("/tmp/outside")), ("foreign root", zipfile.ZipInfo("Other.app/x"))):
            with self.subTest(label=label):
                self.f.bundle()
                self.f.add_entry(info, b"/etc/passwd")
                self.unsigned = self.f.input.read_bytes()
                result = self.f.sign()
                self.assert_refused_untouched(result)
                self.assertNotIn("ditto", [call[0] for call in self.f.calls()], "refused before unpacking")

    def test_a_rejected_notarization_publishes_nothing_and_removes_the_keychain(self):
        result = self.f.sign(SHIM_NOTARY="Invalid")
        self.assert_refused_untouched(result, signed=True)
        self.assertIn(["xcrun", "notarytool", "log"], [call[:3] for call in self.f.calls()])
        self.assertNotIn(["xcrun", "stapler", "staple"], [call[:3] for call in self.f.calls()])


class Verify(unittest.TestCase):
    def setUp(self):
        self.f = Fixture(self)

    def verify(self, *args, **env):
        return self.f.run("nightly-verify.sh", "--input", self.f.input, *args, **{name: None for name in SECRETS}, **env)

    def test_asks_the_gui_its_version_with_an_empty_environment(self):
        self.f.bundle(gui=f"#!/bin/sh\n[ -z \"$INHERITED\" ] || exit 9\necho 'FigCraft {VERSION} (nightly-v1)'\n")
        result = self.verify("--expect", "unsigned", INHERITED="generated")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.f.bundle(gui="#!/bin/sh\necho 'FigCraft 0.18.1 (stable)'\n")
        self.assertNotEqual(self.verify("--expect", "unsigned").returncode, 0)

    def test_the_launcher_profile_form_must_answer_promptly_and_cleanly(self):
        answer = f"echo 'FigCraft {VERSION} (nightly-v1)'"
        # The shared producer smoke's exact form, with a fresh profile directory.
        exact = ('#!/bin/sh\nif [ "$1" = --launch-profile ]; then\n'
                 '  [ "$#" = 5 ] && [ "$2" = nightly-v1 ] && [ "$3" = --profile-root ] && [ -d "$4" ] && [ "$5" = --version ] || exit 7\n'
                 f'fi\n{answer}\n')
        self.f.bundle(gui=exact)
        result = self.verify("--expect", "unsigned")
        self.assertEqual(result.returncode, 0, result.stdout)
        hangs = f'#!/bin/sh\n[ "$1" != --launch-profile ] || exec /bin/sleep 60\n{answer}\n'
        self.f.bundle(gui=hangs)
        started = time.monotonic()
        result = self.verify("--expect", "unsigned")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("within 15 s", result.stdout)
        self.assertLess(time.monotonic() - started, 45, "a hanging query is ended, not waited for")
        noisy = f'#!/bin/sh\n[ "$1" != --launch-profile ] || echo refusal >&2\n{answer}\n'
        self.f.bundle(gui=noisy)
        result = self.verify("--expect", "unsigned")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("standard error", result.stdout)

    def test_a_signed_check_needs_the_signature_ticket_and_team(self):
        self.f.bundle()
        self.assertNotEqual(self.verify("--expect", "signed").returncode, 0, "no team to check against")
        self.assertNotEqual(self.verify("--expect", "signed", "--team", TEAM).returncode, 0, "no stapled ticket")
        self.f.bundle(extra={"Contents/CodeResources": b"ticket"})
        self.assertEqual(self.verify("--expect", "signed", "--team", TEAM).returncode, 0)
        self.assertNotEqual(self.verify("--expect", "signed", "--team", TEAM, SHIM_TEAM="ZZZZZ99999").returncode, 0)

    def test_refuses_to_run_the_payload_beside_signing_material(self):
        self.f.bundle()
        for name in ("APPLE_PASSWORD", "APPLE_CERTIFICATE", "MACOS_KEYCHAIN"):
            with self.subTest(name=name):
                result = self.f.run("nightly-verify.sh", "--input", self.f.input, "--expect", "unsigned",
                                    **{**{key: None for key in SECRETS}, name: "generated"})
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(self.f.ran.exists())
                self.assertEqual(self.f.calls(), [])


class Build(unittest.TestCase):
    def test_compilation_refuses_signing_material_before_building(self):
        f = Fixture(self)
        for name in ("APPLE_CERTIFICATE", "APPLE_PASSWORD", "MACOS_SIGN_IDENTITY", "WINDOWS_CERTIFICATE", "AZURE_CLIENT_SECRET", "LINUX_SIGNING_KEY"):
            with self.subTest(name=name):
                result = f.run("nightly.sh", "--arch", "aarch64", **{**{key: None for key in SECRETS}, name: "generated"},
                               FIGCRAFT_BUILD_CHANNEL="nightly-v1", FIGCRAFT_VERSION=VERSION)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(name, result.stdout)
                self.assertNotIn("cargo", [call[0] for call in f.calls()])


if __name__ == "__main__":
    unittest.main()
