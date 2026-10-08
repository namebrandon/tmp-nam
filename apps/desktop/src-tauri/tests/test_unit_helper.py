"""Host tests for the device helper; all storage is isolated from the unit."""
import contextlib
import errno
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock


HELPER = Path(__file__).resolve().parents[1] / "src" / "unit_helper.py"


class PlayerOptionsTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.path = Path(directory.name) / "player.json"
        spec = importlib.util.spec_from_file_location("unit_helper", str(HELPER))
        self.helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.helper)
        self.helper.PLAYER = str(self.path)

    def save(self, value):
        self.path.write_text(json.dumps(value))

    def options(self, size="0.5", gain="-", sha="new-hash"):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.helper.cmd_opts([sha, size, gain])
        return json.loads(output.getvalue())

    def assert_rejected_without_write(self, size="0.5", gain="-"):
        before = self.path.read_bytes()
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            with self.assertRaises((SystemExit, OSError, ValueError)):
                self.helper.cmd_opts(["new-hash", size, gain])
        self.assertEqual(self.path.read_bytes(), before)
        self.assertEqual(output.getvalue(), "")

    def test_missing_settings_are_initialized(self):
        self.assertEqual(self.options(), {"options": {"size": 0.5}})
        self.assertEqual(json.loads(self.path.read_text()),
                         {"models": {"new-hash": {"size": 0.5}}})

    def test_malformed_settings_are_preserved(self):
        for raw in (b'{"models":{"existing":', b'', b'not json', b'\xff'):
            with self.subTest(raw=raw):
                self.path.write_bytes(raw)
                self.assert_rejected_without_write()

    def test_invalid_settings_structure_is_preserved(self):
        for value in (None, [], "settings", {}, {"models": None},
                      {"models": []}, {"models": "invalid"}):
            with self.subTest(value=value):
                self.save(value)
                self.assert_rejected_without_write()

    def test_invalid_selected_entry_is_preserved(self):
        for entry in (None, [], "invalid", 0):
            with self.subTest(entry=entry):
                self.save({"models": {"new-hash": entry}})
                self.assert_rejected_without_write()

    def test_unreadable_settings_are_preserved(self):
        self.save({"models": {"existing": {"size": 0.25}}})
        real_open = open
        for code in (errno.EACCES, errno.EIO):
            def fail_read(path, mode="r", *args, **kwargs):
                if path == str(self.path) and mode == "r":
                    raise OSError(code, os.strerror(code), path)
                return real_open(path, mode, *args, **kwargs)

            with self.subTest(errno=code):
                # Inject the read error: chmod is unreliable when run as root.
                with mock.patch.object(self.helper, "open", fail_read, create=True):
                    self.assert_rejected_without_write()

    def test_missing_symlink_target_does_not_replace_existing_link(self):
        self.path.symlink_to(self.path.parent / "missing.json")
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            with self.assertRaises((SystemExit, OSError, ValueError)):
                self.helper.cmd_opts(["new-hash", "0.5", "-"])
        self.assertTrue(self.path.is_symlink())
        self.assertEqual(output.getvalue(), "")

    def test_valid_update_preserves_other_models_and_fields(self):
        value = {"models": {"existing": {"size": 0.25},
                            "new-hash": {"sample_rate_hz": 48000}},
                 "unrelated": {"keep": True}}
        self.save(value)
        self.assertEqual(self.options(gain="1.25"),
                         {"options": {"size": 0.5, "output_gain": 1.25,
                                      "sample_rate_hz": 48000}})
        value["models"]["new-hash"].update(size=0.5, output_gain=1.25)
        self.assertEqual(json.loads(self.path.read_text()), value)

    def test_removing_options_preserves_other_fields(self):
        self.save({"models": {"new-hash": {"size": 0.5, "output_gain": 1.25,
                                           "sample_rate_hz": 48000}}})
        self.assertEqual(self.options(size="-"),
                         {"options": {"sample_rate_hz": 48000}})
        self.assertEqual(json.loads(self.path.read_text()),
                         {"models": {"new-hash": {"sample_rate_hz": 48000}}})

    def test_removing_last_options_removes_only_selected_entry(self):
        self.save({"models": {"new-hash": {"size": 0.5},
                              "existing": {"output_gain": 1.25}}})
        self.assertEqual(self.options(size="-"), {"options": {}})
        self.assertEqual(json.loads(self.path.read_text()),
                         {"models": {"existing": {"output_gain": 1.25}}})

    def test_invalid_option_does_not_change_settings(self):
        self.save({"models": {"existing": {"size": 0.25}}})
        self.assert_rejected_without_write(gain="not-a-number")

    def test_failed_atomic_save_preserves_existing_settings(self):
        self.save({"models": {"existing": {"size": 0.25}}})
        for operation in ("fsync", "rename"):
            with self.subTest(operation=operation):
                with mock.patch.object(self.helper.os, operation,
                                       side_effect=OSError(errno.ENOSPC, "full")):
                    self.assert_rejected_without_write()


if __name__ == "__main__":
    unittest.main()
