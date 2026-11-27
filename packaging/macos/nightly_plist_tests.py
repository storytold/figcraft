#!/usr/bin/env python3
"""Generated metadata/sentinel fixtures; no native binary, signature, archive or network."""
import importlib.util
import os
from pathlib import Path
import plistlib
import runpy
import sys
from unittest import mock
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("fig_nightly_plist", HERE / "nightly_plist.py")
POLICY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(POLICY)
COMMIT = "a" * 40
VERSION = "0.18.1-nightly.20261124.12345.1.g" + COMMIT[:12]
# The template keys this app carries: its own version and commit keys, and these platform ones.
PLATFORM_KEYS = ("LSMinimumSystemVersion", "NSHighResolutionCapable", "NSHumanReadableCopyright", "LSApplicationCategoryType")


class NightlyMetadata(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()
        previous = Path.cwd()
        os.chdir(self.root)
        self.addCleanup(os.chdir, previous)
        self.template = self.root / "Info.plist.in"
        self.original = (HERE / "Info.plist.in").read_bytes()
        self.template.write_bytes(self.original)
        self.output = self.root / "Info.plist"

    def test_nightly_transform_removes_associations_without_changing_stable_or_platform_metadata(self):
        stable = plistlib.loads(self.original)
        stable["CFBundleURLTypes"] = [{"CFBundleURLSchemes": ["generated-test-scheme"]}]
        prepared = plistlib.dumps(stable)
        self.template.write_bytes(prepared)
        POLICY.write_plist(self.template, self.output, VERSION, COMMIT)
        rendered = plistlib.loads(self.output.read_bytes())
        for key in POLICY.ASSOCIATIONS:
            self.assertNotIn(key, rendered)
        self.assertNotEqual(rendered["CFBundleIdentifier"], stable["CFBundleIdentifier"])
        self.assertEqual(rendered["CFBundleIdentifier"], "ai.storyteller.figcraft.nightly")
        self.assertEqual(rendered["CFBundleExecutable"], "FigCraft")
        self.assertEqual(rendered["CFBundleIconFile"], "figcraft-nightly")
        self.assertEqual(rendered["FigCraftVersion"], VERSION)
        self.assertEqual(rendered["FigCraftBuildCommit"], COMMIT)
        self.assertTrue(VERSION.startswith(rendered["CFBundleVersion"] + "-nightly."))
        for key in PLATFORM_KEYS:
            self.assertEqual(rendered[key], stable[key])
        self.assertEqual(self.template.read_bytes(), prepared)
        self.assertEqual((HERE / "Info.plist.in").read_bytes(), self.original)

    def test_bad_identity_or_template_refuses_before_creating_output(self):
        for version, commit in (("0.18.1", COMMIT), (VERSION, "b" * 40), (VERSION, "not-a-commit"), (VERSION.replace("20261124", "20261399"), COMMIT)):
            with self.subTest(version=version, commit=commit):
                with self.assertRaises(ValueError):
                    POLICY.write_plist(self.template, self.output, version, commit)
                self.assertFalse(self.output.exists())
                self.assertEqual(self.template.read_bytes(), self.original)
        self.template.write_bytes(plistlib.dumps({"CFBundlePackageType": "generated-not-an-app"}))
        with self.assertRaises(ValueError):
            POLICY.write_plist(self.template, self.output, VERSION, COMMIT)
        self.assertFalse(self.output.exists())

    def test_cli_writes_the_requested_identity_without_replacing_the_template(self):
        helper = HERE / "nightly_plist.py"
        argv = [str(helper), "--template", str(self.template), "--output", str(self.output), "--version", VERSION, "--source-commit", COMMIT]
        with mock.patch.object(sys, "argv", argv):
            runpy.run_path(str(helper), run_name="__main__")
        result = plistlib.loads(self.output.read_bytes())
        self.assertEqual(result["FigCraftVersion"], VERSION)
        self.assertEqual(result["FigCraftBuildCommit"], COMMIT)
        self.assertEqual(self.template.read_bytes(), self.original)

    def test_template_alias_or_nonfile_refuses_before_output(self):
        alias = self.root / "template-alias"
        alias.symlink_to(self.template)
        for template in (alias, self.root, self.root / "missing-template"):
            with self.subTest(template=template), self.assertRaises(ValueError):
                POLICY.write_plist(template, self.output, VERSION, COMMIT)
            self.assertFalse(self.output.exists())
            self.assertEqual(self.template.read_bytes(), self.original)

    def test_failed_write_cleans_owned_output_but_preserves_a_replacement(self):
        sentinel = b"generated replacement artifact"
        for disposition in ("owned", "missing", "replacement"):
            with self.subTest(disposition=disposition):
                if self.output.exists():
                    self.output.unlink()

                def fail_sync(_descriptor):
                    # macOS/Linux allow unlink of an open file: exercise the real
                    # cleanup ownership check, not a mocked inode comparison.
                    if disposition != "owned":
                        self.output.unlink()
                    if disposition == "replacement":
                        self.output.write_bytes(sentinel)
                    raise OSError("generated fsync failure")

                with mock.patch.object(POLICY.os, "fsync", side_effect=fail_sync), self.assertRaises(OSError):
                    POLICY.write_plist(self.template, self.output, VERSION, COMMIT)
                if disposition == "replacement":
                    self.assertEqual(self.output.read_bytes(), sentinel)
                else:
                    self.assertFalse(self.output.exists())
                self.assertEqual(self.template.read_bytes(), self.original)

    def test_exclusive_output_preserves_existing_bytes_and_symlink_targets(self):
        sentinel = b"generated owned sentinel, not an existing app or user file"
        self.output.write_bytes(sentinel)
        with self.assertRaises(FileExistsError):
            POLICY.write_plist(self.template, self.output, VERSION, COMMIT)
        self.assertEqual(self.output.read_bytes(), sentinel)
        self.output.unlink()
        external = self.root / "sentinel"
        external.write_bytes(sentinel)
        self.output.symlink_to(external)
        with self.assertRaisesRegex(ValueError, "symlinks or aliases"):
            POLICY.write_plist(self.template, self.output, VERSION, COMMIT)
        self.assertEqual(external.read_bytes(), sentinel)
        self.assertTrue(self.output.is_symlink())

    def test_template_outside_build_root_refuses_before_output(self):
        with tempfile.TemporaryDirectory() as external:
            template = Path(external) / "Info.plist.in"
            template.write_bytes(self.original)
            with self.assertRaises(ValueError):
                POLICY.write_plist(template, self.output, VERSION, COMMIT)
            self.assertFalse(self.output.exists())
            self.assertEqual(template.read_bytes(), self.original)

    def test_output_outside_build_root_refuses_before_write(self):
        with tempfile.TemporaryDirectory() as external:
            output = Path(external) / "Info.plist"
            with self.assertRaises(ValueError):
                POLICY.write_plist(self.template, output, VERSION, COMMIT)
            self.assertFalse(output.exists())

    def test_parent_traversal_refuses_even_to_an_owned_sibling(self):
        with tempfile.TemporaryDirectory(dir=self.root.parent) as external:
            output = Path("..") / Path(external).name / "Info.plist"
            with self.assertRaises(ValueError):
                POLICY.write_plist(self.template, output, VERSION, COMMIT)
            self.assertFalse((Path(external) / "Info.plist").exists())

    def test_symlinked_output_parent_refuses(self):
        with tempfile.TemporaryDirectory() as external:
            alias = self.root / "alias"
            alias.symlink_to(external, target_is_directory=True)
            with self.assertRaises(ValueError):
                POLICY.write_plist(self.template, alias / "Info.plist", VERSION, COMMIT)
            self.assertFalse((Path(external) / "Info.plist").exists())

    def test_template_read_is_bounded_before_output(self):
        self.template.write_bytes(plistlib.dumps({"CFBundlePackageType": "APPL", "fixture": "x" * 300000}))
        with self.assertRaises(ValueError):
            POLICY.write_plist(self.template, self.output, VERSION, COMMIT)
        self.assertFalse(self.output.exists())

    def test_nonprivate_build_root_refuses(self):
        self.root.chmod(0o755)
        try:
            with self.assertRaises(ValueError):
                POLICY.write_plist(self.template, self.output, VERSION, COMMIT)
            self.assertFalse(self.output.exists())
        finally:
            self.root.chmod(0o700)

    def test_output_keeps_normal_new_file_permissions_inside_private_root(self):
        previous = os.umask(0o022)
        try:
            reference = self.root / "reference"
            reference.write_bytes(b"generated ordinary output")
            POLICY.write_plist(self.template, self.output, VERSION, COMMIT)
        finally:
            os.umask(previous)
        self.assertEqual(self.output.stat().st_mode & 0o777, reference.stat().st_mode & 0o777)

    def test_open_parent_anchor_prevents_a_later_alias_redirect(self):
        parent = self.root / "stage"
        parent.mkdir()
        retained = self.root / "retained-stage"
        destination = parent / "Info.plist"
        original = POLICY.OwnedBuildRoot._parent
        with tempfile.TemporaryDirectory() as external:

            def swap_after_parent_open(root, path):
                descriptor, name = original(root, path)
                if Path(path) == destination:
                    parent.rename(retained)
                    parent.symlink_to(external, target_is_directory=True)
                return descriptor, name

            with mock.patch.object(POLICY.OwnedBuildRoot, "_parent", swap_after_parent_open):
                POLICY.write_plist(self.template, destination, VERSION, COMMIT)
            self.assertFalse((Path(external) / "Info.plist").exists())
            self.assertEqual(plistlib.loads((retained / "Info.plist").read_bytes())["FigCraftVersion"], VERSION)


if __name__ == "__main__":
    unittest.main()
