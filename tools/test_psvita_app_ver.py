"""Unit tests for PS Vita SFO APP_VER mapping: python3 -I tools/test_psvita_app_ver.py."""
import importlib.util
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import unittest

REPO_ROOT = Path(__file__).resolve().parents[1]
APP_VER_SCRIPT = REPO_ROOT / "platforms" / "psvita" / "scripts" / "app_ver.py"
BUILD_PLAYER_SCRIPT = REPO_ROOT / "platforms" / "psvita" / "scripts" / "build-player.sh"

spec = importlib.util.spec_from_file_location("psvita_app_ver", APP_VER_SCRIPT)
app_ver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(app_ver)


def run_cli(**env_overrides):
    env = {"PATH": os.environ.get("PATH", ""), "PYTHONDONTWRITEBYTECODE": "1"}
    env.update(env_overrides)
    return subprocess.run(
        [sys.executable, "-I", str(APP_VER_SCRIPT)],
        cwd=REPO_ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )


class VitaAppVerTests(unittest.TestCase):
    def test_default_and_known_anchors(self):
        self.assertEqual(app_ver.app_ver_from_env({}), "00.20")
        self.assertEqual(app_ver.app_ver_from_env({"CMVS_APP_VER": ""}), "00.20")
        self.assertEqual(app_ver.resolve_app_ver("0.0.0"), "00.00")
        self.assertEqual(app_ver.resolve_app_ver("0.1.0"), "00.20")
        self.assertEqual(app_ver.resolve_app_ver("0.1.9"), "00.29")
        self.assertEqual(app_ver.resolve_app_ver("0.1.10"), "00.30")
        self.assertEqual(app_ver.resolve_app_ver("0.9.9"), "01.89")
        self.assertEqual(app_ver.resolve_app_ver("0.10.0"), "02.00")
        self.assertEqual(app_ver.resolve_app_ver("1.0.0"), "10.00")
        self.assertEqual(app_ver.resolve_app_ver("1.0.1"), "10.01")
        self.assertEqual(app_ver.resolve_app_ver("9.49.19"), "99.99")

    def test_specific_monotonicity_regressions(self):
        for older, newer in [
            ("0.1.9", "0.1.10"),
            ("0.9.9", "0.10.0"),
            ("1.0.0", "1.0.1"),
            ("0.49.19", "1.0.0"),
        ]:
            with self.subTest(older=older, newer=newer):
                self.assertLess(
                    app_ver.resolve_app_ver(older),
                    app_ver.resolve_app_ver(newer),
                )

    def test_exhaustive_bijective_monotonicity(self):
        sfo_re = re.compile(r"\A[0-9]{2}\.[0-9]{2}\Z")
        outputs = []
        previous = None
        for major in range(10):
            for minor in range(50):
                for patch in range(20):
                    rendered = app_ver.resolve_app_ver(f"{major}.{minor}.{patch}")
                    self.assertRegex(rendered, sfo_re)
                    if previous is not None:
                        self.assertLess(previous, rendered)
                    previous = rendered
                    outputs.append(rendered)

        self.assertEqual(len(outputs), 10_000)
        self.assertEqual(len(set(outputs)), 10_000)
        self.assertEqual(outputs[0], "00.00")
        self.assertEqual(outputs[-1], "99.99")

    def test_explicit_app_ver_override_and_validation(self):
        self.assertEqual(app_ver.resolve_app_ver("10.0.0", app_ver="12.34"), "12.34")
        self.assertEqual(
            app_ver.app_ver_from_env(
                {"CMVS_VERSION": "0.1.0-rc.1", "CMVS_APP_VER": "00.19"}
            ),
            "00.19",
        )
        self.assertEqual(app_ver.resolve_app_ver("0.1.0", app_ver="00.00"), "00.00")
        self.assertEqual(app_ver.resolve_app_ver("0.1.0", app_ver="99.99"), "99.99")

        for bad in ["1.2", "01.000", "abc", "1234", "12.34\n", "   ", "-1.00", "١٢.٣٤"]:
            with self.subTest(app_ver=bad):
                with self.assertRaises(ValueError):
                    app_ver.resolve_app_ver("0.1.0", app_ver=bad)

    def test_out_of_range_and_malformed_versions_fail(self):
        for bad in [
            "0.1.20",
            "0.50.0",
            "10.0.0",
            "9.49.20",
            "9.50.0",
            "0.1",
            "1",
            "",
            "0.1.0-rc.1",
            "0.1.0+build.1",
            "v0.1.0",
            " 0.1.0",
            "0.1.0 ",
            "0.1.0\n",
            "01.1.0",
            "0.01.0",
            "0.1.00",
            "٠.١.٠",
        ]:
            with self.subTest(version=bad):
                with self.assertRaises(ValueError) as ctx:
                    app_ver.resolve_app_ver(bad)
                self.assertIn("CMVS_APP_VER=XX.YY", str(ctx.exception))

    def test_cli_exit_codes_and_stderr(self):
        ok = run_cli(CMVS_VERSION="0.1.10", CMVS_APP_VER="")
        self.assertEqual(ok.returncode, 0, msg=ok.stderr)
        self.assertEqual(ok.stdout, "00.30\n")
        self.assertEqual(ok.stderr, "")

        override = run_cli(CMVS_VERSION="10.0.0", CMVS_APP_VER="12.34")
        self.assertEqual(override.returncode, 0, msg=override.stderr)
        self.assertEqual(override.stdout, "12.34\n")

        for env in [
            {"CMVS_VERSION": "0.1.20"},
            {"CMVS_VERSION": "0.50.0"},
            {"CMVS_VERSION": "10.0.0"},
            {"CMVS_VERSION": "0.1"},
            {"CMVS_VERSION": "0.1.0-rc.1"},
            {"CMVS_VERSION": "v0.1.0"},
            {"CMVS_APP_VER": "1.2"},
            {"CMVS_APP_VER": "01.000"},
            {"CMVS_APP_VER": "abc"},
        ]:
            with self.subTest(env=env):
                failed = run_cli(**env)
                self.assertNotEqual(failed.returncode, 0)
                self.assertEqual(failed.stdout, "")
                self.assertIn("CMVS_APP_VER", failed.stderr)

    @unittest.skipUnless(
        os.name != "nt" and shutil.which("sh") and shutil.which("python3"),
        "POSIX sh and python3 required",
    )
    def test_build_player_fails_fast_before_compilation(self):
        script_text = BUILD_PLAYER_SCRIPT.read_text(encoding="utf-8")
        app_ver_pos = script_text.index("platforms/psvita/scripts/app_ver.py")
        gcc_pos = script_text.index("arm-vita-eabi-gcc")
        cargo_pos = script_text.index("cargo ")
        self.assertLess(app_ver_pos, gcc_pos)
        self.assertLess(app_ver_pos, cargo_pos)

        for env_extra in [
            {"CMVS_VERSION": "0.1.0-rc.1"},
            {"CMVS_VERSION": "10.0.0"},
            {"CMVS_APP_VER": "1.2"},
        ]:
            with self.subTest(env=env_extra):
                env = {
                    "PATH": os.environ.get("PATH", ""),
                    "PYTHONDONTWRITEBYTECODE": "1",
                }
                env.update(env_extra)
                proc = subprocess.run(
                    ["sh", str(BUILD_PLAYER_SCRIPT)],
                    cwd=REPO_ROOT,
                    env=env,
                    capture_output=True,
                    text=True,
                    check=False,
                )
                self.assertNotEqual(proc.returncode, 0)
                self.assertIn("app_ver.py: error:", proc.stderr)
                self.assertNotIn("arm-vita-eabi-gcc", proc.stderr)


if __name__ == "__main__":
    unittest.main()
