import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "distribution" / "scripts"
sys.path.insert(0, str(SCRIPTS))

from common import archive_name, host_target, read_json, release_manifest, sha256
from verify_channels import channel_diagnostic_environments, write_npx_shim


def invoke(*arguments, cwd=None):
    return subprocess.run(
        [str(value) for value in arguments],
        cwd=str(cwd) if cwd else None,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


class DistributionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = Path(tempfile.mkdtemp(prefix="nostdb-distribution-test-"))

    def tearDown(self):
        shutil.rmtree(self.temporary)

    def fake_binary(self):
        if sys.platform == "win32":
            binary = self.temporary / "nostdb.cmd"
            binary.write_text(
                "@echo off\n"
                "if \"%~1\"==\"--version\" (\n"
                "  echo nostdb 0.0.3\n"
                "  exit /b 0\n"
                ")\n"
                "if \"%~1\"==\"--help\" (\n"
                "  echo Usage: nostdb COMMAND\n"
                "  exit /b 0\n"
                ")\n"
                "exit /b 2\n",
                encoding="utf-8",
            )
            return binary
        binary = self.temporary / "nostdb"
        binary.write_text(
            "#!{}\n"
            "import sys\n"
            "if sys.argv[1:] == ['--version']:\n"
            "    print('nostdb 0.0.3')\n"
            "elif sys.argv[1:] == ['--help']:\n"
            "    print('Usage: nostdb COMMAND')\n"
            "else:\n"
            "    sys.exit(2)\n".format(sys.executable),
            encoding="utf-8",
        )
        binary.chmod(0o755)
        return binary

    def test_channel_diagnostics_explicitly_authorize_each_installed_binary(self):
        npx_environment = {"PATH": "npx-only"}
        environments = channel_diagnostic_environments(
            Path("/candidate/direct/nostdb"),
            Path("/candidate/npm/bin/nostdb"),
            npx_environment,
            Path("/candidate/homebrew/nostdb"),
        )
        self.assertIs(environments["npx"], npx_environment)
        self.assertEqual(
            environments["direct"]["NOSTDB_BIN"], "/candidate/direct/nostdb"
        )
        self.assertEqual(
            environments["npm_global"]["NOSTDB_BIN"],
            "/candidate/npm/bin/nostdb",
        )
        self.assertEqual(
            environments["homebrew"]["NOSTDB_BIN"],
            "/candidate/homebrew/nostdb",
        )

    @unittest.skipIf(os.name == "nt", "POSIX npx shim behavior")
    def test_npx_channel_accepts_only_the_latest_selector(self):
        shim_directory = self.temporary / "npx-shim"
        shim_directory.mkdir()
        real_npx = self.temporary / "real-npx"
        real_npx.write_text(
            "#!{}\n"
            "import json, os, sys\n"
            "open(os.environ['NPX_LOG'], 'w').write(json.dumps(sys.argv[1:]))\n"
            .format(sys.executable),
            encoding="utf-8",
        )
        real_npx.chmod(0o755)
        prefix = self.temporary / "npm-prefix"
        write_npx_shim(shim_directory, str(real_npx), prefix)
        environment = os.environ.copy()
        log = self.temporary / "npx.json"
        environment.update(
            {
                "NPX_LOG": str(log),
                "NPX_PREFIX": str(prefix),
                "REAL_NPX": str(real_npx),
            }
        )
        accepted = subprocess.run(
            [
                str(shim_directory / "npx"),
                "--yes",
                "--package=@nostdb/cli@latest",
                "nostdb",
                "--version",
            ],
            check=False,
            env=environment,
        )
        self.assertEqual(accepted.returncode, 0)
        self.assertEqual(
            json.loads(log.read_text(encoding="utf-8")),
            [
                "--yes",
                "--offline",
                "--prefix",
                str(prefix),
                "nostdb",
                "--version",
            ],
        )
        refused = subprocess.run(
            [
                str(shim_directory / "npx"),
                "--yes",
                "--package=@nostdb/cli@0.0.3",
                "nostdb",
                "--version",
            ],
            check=False,
            env=environment,
            stderr=subprocess.PIPE,
            text=True,
        )
        self.assertEqual(refused.returncode, 97)
        self.assertIn("unexpected latest npx command", refused.stderr)

    def metadata(self):
        metadata = self.temporary / "metadata"
        metadata.mkdir()
        (metadata / "SBOM.spdx.json").write_text(
            '{"spdxVersion":"SPDX-2.3"}\n', encoding="utf-8"
        )
        (metadata / "THIRD_PARTY_LICENSES.json").write_text(
            '{"metadata_version":1}\n', encoding="utf-8"
        )
        return metadata

    def native_evidence(self, target):
        evidence = self.temporary / (target + ".json")
        evidence.write_text(
            json.dumps(
                {
                    "evidence_version": 1,
                    "host_target": target,
                    "passed": True,
                    "target": target,
                    "translated": False,
                    "tests": {
                        "help": {"returncode": 0, "stdout": "Usage: nostdb\n"},
                        "version": {
                            "returncode": 0,
                            "stdout": "nostdb 0.0.3\n",
                        },
                    },
                    "version": "0.0.3",
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        return evidence

    def test_manifest_declares_six_native_targets(self):
        manifest = release_manifest()
        self.assertEqual(manifest["version"], "0.0.3")
        self.assertEqual(
            set(manifest["targets"]),
            {
                "aarch64-apple-darwin",
                "x86_64-apple-darwin",
                "aarch64-pc-windows-msvc",
                "x86_64-pc-windows-msvc",
                "aarch64-unknown-linux-gnu",
                "x86_64-unknown-linux-gnu",
            },
        )
        self.assertEqual(
            len({details["npm_package"] for details in manifest["targets"].values()}),
            6,
        )

    def test_native_smoke_and_deterministic_archive(self):
        target = host_target()
        binary = self.fake_binary()
        evidence = self.temporary / "native.json"
        smoke = invoke(
            sys.executable,
            SCRIPTS / "smoke_candidate.py",
            "--target",
            target,
            "--binary",
            binary,
            "--output",
            evidence,
        )
        self.assertEqual(smoke.returncode, 0, smoke.stderr)
        self.assertTrue(read_json(evidence)["passed"])
        outputs = []
        for name in ("first", "second"):
            output = self.temporary / name
            assembled = invoke(
                sys.executable,
                SCRIPTS / "assemble_candidate.py",
                "--target",
                target,
                "--binary",
                binary,
                "--native-evidence",
                evidence,
                "--metadata",
                self.metadata() if name == "first" else self.temporary / "metadata",
                "--output",
                output,
            )
            self.assertEqual(assembled.returncode, 0, assembled.stderr)
            archive = Path(assembled.stdout.strip())
            verified = invoke(
                sys.executable,
                SCRIPTS / "verify_candidate.py",
                "--archive",
                archive,
            )
            self.assertEqual(verified.returncode, 0, verified.stderr)
            outputs.append(archive)
        self.assertEqual(sha256(outputs[0]), sha256(outputs[1]))

    def test_archive_refuses_non_native_or_incomplete_evidence(self):
        target = host_target()
        evidence = self.native_evidence(target)
        payload = read_json(evidence)
        payload["host_target"] = next(
            candidate for candidate in release_manifest()["targets"] if candidate != target
        )
        evidence.write_text(json.dumps(payload), encoding="utf-8")
        result = invoke(
            sys.executable,
            SCRIPTS / "assemble_candidate.py",
            "--target",
            target,
            "--binary",
            self.fake_binary(),
            "--native-evidence",
            evidence,
            "--metadata",
            self.metadata(),
            "--output",
            self.temporary / "refused",
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("native evidence host_target mismatch", result.stderr)

    def test_stages_unpublished_npm_launcher_and_native_package(self):
        target = host_target()
        output = self.temporary / "npm"
        binary = self.fake_binary()
        result = invoke(
            sys.executable,
            SCRIPTS / "stage_npm_candidate.py",
            "--target",
            target,
            "--binary",
            binary,
            "--output",
            output,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        payload = json.loads(result.stdout)
        self.assertFalse(payload["published"])
        self.assertEqual(payload["launcher"]["name"], "@nostdb/cli")
        launcher = output / payload["launcher"]["filename"]
        platform = output / payload["platform"]["filename"]
        self.assertTrue(launcher.is_file())
        self.assertTrue(platform.is_file())
        for archive in (launcher, platform):
            with tarfile.open(archive, mode="r:gz") as package:
                contents = {
                    member.name: package.extractfile(member).read()
                    for member in package.getmembers()
                    if member.isfile()
                }
            self.assertEqual(contents["package/LICENSE"], (ROOT / "LICENSE").read_bytes())
            self.assertEqual(contents["package/NOTICE"], (ROOT / "NOTICE").read_bytes())
            self.assertEqual(contents["package/README.md"], (ROOT / "README.md").read_bytes())
            if archive == platform:
                executable = "nostdb.exe" if "windows" in target else "nostdb"
                self.assertEqual(
                    contents["package/bin/{}".format(executable)], binary.read_bytes()
                )

    def test_renders_two_architecture_homebrew_formula(self):
        archives = {}
        for target in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
            archive = self.temporary / archive_name("0.0.3", target, "tar.gz")
            archive.write_bytes(target.encode("ascii"))
            record = {
                "archive_sha256": sha256(archive),
                "published": False,
                "target": target,
            }
            archive.with_name(archive.name + ".manifest.json").write_text(
                json.dumps(record), encoding="utf-8"
            )
            archives[target] = archive
        formula = self.temporary / "Formula" / "nostdb.rb"
        rendered = invoke(
            sys.executable,
            SCRIPTS / "render_homebrew.py",
            "--arm64-archive",
            archives["aarch64-apple-darwin"],
            "--x64-archive",
            archives["x86_64-apple-darwin"],
            "--output",
            formula,
        )
        self.assertEqual(rendered.returncode, 0, rendered.stderr)
        text = formula.read_text(encoding="utf-8")
        self.assertIn('version "0.0.3"', text)
        self.assertIn("on_arm do", text)
        self.assertIn("on_intel do", text)
        syntax = invoke("ruby", "-c", formula)
        self.assertEqual(syntax.returncode, 0, syntax.stderr)

    @unittest.skipUnless(sys.platform == "darwin", "Homebrew targets macOS")
    def test_renders_non_publishable_native_host_smoke_formula(self):
        target = host_target()
        archive = self.temporary / archive_name("0.0.3", target, "tar.gz")
        archive.write_bytes(target.encode("ascii"))
        archive.with_name(archive.name + ".manifest.json").write_text(
            json.dumps(
                {
                    "archive_sha256": sha256(archive),
                    "published": False,
                    "target": target,
                }
            ),
            encoding="utf-8",
        )
        formula = self.temporary / "smoke" / "Formula" / "nostdb.rb"
        rendered = invoke(
            sys.executable,
            SCRIPTS / "render_homebrew.py",
            "--host-smoke-archive",
            archive,
            "--output",
            formula,
        )
        self.assertEqual(rendered.returncode, 0, rendered.stderr)
        text = formula.read_text(encoding="utf-8")
        self.assertIn("HOST-ONLY SMOKE FORMULA. NEVER PUBLISH.", text)
        self.assertEqual(text.count(archive.resolve().as_uri()), 2)
        syntax = invoke("ruby", "-c", formula)
        self.assertEqual(syntax.returncode, 0, syntax.stderr)


if __name__ == "__main__":
    unittest.main()
