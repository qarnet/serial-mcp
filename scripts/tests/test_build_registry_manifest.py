"""Offline, deterministic unit tests for scripts/build_registry_manifest.py.

Run from anywhere:
    python3 -m unittest discover -s scripts/tests -v
"""

import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import build_registry_manifest as bm

PLATFORMS = bm.EXPECTED_ASSETS
VERSION = "9.8.7"
TAG = f"v{VERSION}"

TEMPLATE = {
    "name": "serial-mcp",
    "description": "MCP server for serial ports (27 tools).",
    "version": VERSION,
}


def write_template(directory, version=VERSION, with_packages=False):
    doc = dict(TEMPLATE)
    doc["version"] = version
    if with_packages:
        doc["packages"] = [{"registryType": "mcpb"}]
    path = Path(directory) / "server.json"
    path.write_text(json.dumps(doc), encoding="utf-8")
    return path


def write_asset(directory, name, size):
    path = Path(directory) / name
    path.write_bytes(b"\x00" * size)
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    return path, digest


def make_metadata(directory, assets, tag=TAG):
    meta = {"tag": tag, "assets": assets}
    path = Path(directory) / "release-metadata.json"
    path.write_text(json.dumps(meta), encoding="utf-8")
    return path


def happy_fixture(base):
    """Directory with template + four real assets + matching metadata."""
    assets = Path(base) / "assets"
    assets.mkdir()
    entries = []
    for name in PLATFORMS:
        _, digest = write_asset(assets, name, size=1024)
        entries.append({"name": name, "size": 1024, "digest": f"sha256:{digest}"})
    template = write_template(base)
    metadata = make_metadata(base, entries)
    return template, metadata, assets


def expected_urls(version=VERSION):
    return [
        f"https://github.com/{bm.REPO}/releases/download/v{version}/{name}"
        for name in PLATFORMS
    ]


class ManifestBuilderTests(unittest.TestCase):
    def test_happy_path_exact_manifest(self):
        with tempfile.TemporaryDirectory() as base:
            template, metadata, assets = happy_fixture(base)
            manifest = bm.build(str(template), VERSION, str(metadata), str(assets))
            self.assertEqual(manifest["name"], "serial-mcp")
            self.assertEqual(manifest["description"], TEMPLATE["description"])
            self.assertEqual(manifest["version"], VERSION)
            pkgs = manifest["packages"]
            self.assertEqual([p["identifier"] for p in pkgs], expected_urls())
            for p, name in zip(pkgs, PLATFORMS):
                self.assertEqual(p["registryType"], "mcpb")
                self.assertEqual(p["version"], VERSION)
                self.assertEqual(p["transport"], {"type": "stdio"})
                local = hashlib.sha256((assets / name).read_bytes()).hexdigest()
                self.assertEqual(p["fileSha256"], local)

    def test_happy_path_writes_atomic_output(self):
        with tempfile.TemporaryDirectory() as base:
            template, metadata, assets = happy_fixture(base)
            manifest = bm.build(str(template), VERSION, str(metadata), str(assets))
            out = Path(base) / "out" / "server.json"
            bm.write_atomic(manifest, out)
            self.assertTrue(out.is_file())
            self.assertEqual(json.loads(out.read_text(encoding="utf-8")), manifest)

    def test_happy_path_through_cli(self):
        with tempfile.TemporaryDirectory() as base:
            template, metadata, assets = happy_fixture(base)
            out = Path(base) / "out" / "server.json"
            code = bm.main(
                [
                    "--template",
                    str(template),
                    "--version",
                    VERSION,
                    "--metadata",
                    str(metadata),
                    "--assets",
                    str(assets),
                    "--output",
                    str(out),
                ]
            )
            self.assertEqual(code, 0)
            self.assertTrue(out.is_file())

    def test_validate_version_only_modes(self):
        with tempfile.TemporaryDirectory() as base:
            self.assertEqual(bm.main(["--validate-version-only", "1.2.3"]), 0)
            self.assertEqual(bm.main(["--validate-version-only", "1.2"]), 1)
            self.assertEqual(bm.main(["--validate-version-only", "1.2.3-beta"]), 1)
            self.assertEqual(bm.main(["--validate-version-only", "v1.2.3"]), 1)

    def assert_failure(self, base, version=VERSION, mutate=None, needle=None):
        template, metadata, assets = happy_fixture(base)
        if mutate:
            mutate(template, metadata, assets)
        out = Path(base) / "out" / "server.json"
        with self.assertRaises(bm.BuildError) as ctx:
            bm.build(str(template), version, str(metadata), str(assets))
        self.assertFalse(out.exists(), "failed build must not leave an output file")
        if needle:
            self.assertIn(needle, str(ctx.exception))
        return ctx

    def test_invalid_version_rejected(self):
        with tempfile.TemporaryDirectory() as base:
            self.assert_failure(base, version="1.2", needle="invalid version")

    def test_template_version_mismatch_rejected(self):
        with tempfile.TemporaryDirectory() as base:
            self.assert_failure(
                base,
                version="1.2.3",
                mutate=lambda t, m, a: None,
                needle="does not match requested version",
            )

    def test_template_with_packages_rejected(self):
        with tempfile.TemporaryDirectory() as base:
            self.assert_failure(
                base,
                mutate=lambda t, m, a: t.write_text(
                    json.dumps({**TEMPLATE, "packages": []}), encoding="utf-8"
                ),
                needle="already contains a packages array",
            )

    def test_tag_mismatch_rejected(self):
        with tempfile.TemporaryDirectory() as base:

            def wrong_tag(t, m, a):
                m.write_text(
                    json.dumps({"tag": "v1.2.3", "assets": []}), encoding="utf-8"
                )

            self.assert_failure(base, mutate=wrong_tag, needle="does not match version")

    def test_missing_metadata_asset_rejected(self):
        with tempfile.TemporaryDirectory() as base:

            def drop(t, m, a):
                doc = json.loads(m.read_text(encoding="utf-8"))
                doc["assets"] = doc["assets"][1:]
                m.write_text(json.dumps(doc), encoding="utf-8")

            self.assert_failure(base, mutate=drop, needle="missing expected assets")

    def test_duplicate_metadata_asset_rejected(self):
        with tempfile.TemporaryDirectory() as base:

            def dup(t, m, a):
                doc = json.loads(m.read_text(encoding="utf-8"))
                doc["assets"].append(dict(doc["assets"][0]))
                m.write_text(json.dumps(doc), encoding="utf-8")

            self.assert_failure(base, mutate=dup, needle="duplicate asset")

    def test_unexpected_metadata_asset_rejected(self):
        with tempfile.TemporaryDirectory() as base:

            def extra(t, m, a):
                doc = json.loads(m.read_text(encoding="utf-8"))
                doc["assets"].append(
                    {"name": "unexpected-extra", "size": 1, "digest": "a" * 64}
                )
                m.write_text(json.dumps(doc), encoding="utf-8")

            self.assert_failure(base, mutate=extra, needle="unexpected assets")

    def test_missing_digest_rejected(self):
        with tempfile.TemporaryDirectory() as base:

            def no_digest(t, m, a):
                doc = json.loads(m.read_text(encoding="utf-8"))
                del doc["assets"][0]["digest"]
                m.write_text(json.dumps(doc), encoding="utf-8")

            self.assert_failure(base, mutate=no_digest, needle="digest")

    def test_bad_digest_rejected(self):
        with tempfile.TemporaryDirectory() as base:

            def bad(t, m, a):
                doc = json.loads(m.read_text(encoding="utf-8"))
                doc["assets"][0]["digest"] = "not-a-sha"
                m.write_text(json.dumps(doc), encoding="utf-8")

            self.assert_failure(base, mutate=bad, needle="is not a sha256 digest")

    def test_bare_hash_digest_rejected(self):
        # GitHub emits digests as "sha256:<64 hex>"; a bare 64-hex hash must
        # be rejected with its own explicit error, not silently accepted.
        with tempfile.TemporaryDirectory() as base:

            def bare(t, m, a):
                doc = json.loads(m.read_text(encoding="utf-8"))
                doc["assets"][0]["digest"] = "a" * 64
                m.write_text(json.dumps(doc), encoding="utf-8")

            self.assert_failure(base, mutate=bare, needle="algorithm prefix")

    def test_wrong_digest_rejected(self):
        with tempfile.TemporaryDirectory() as base:

            def wrong(t, m, a):
                doc = json.loads(m.read_text(encoding="utf-8"))
                doc["assets"][0]["digest"] = "sha256:" + ("f" * 64)
                m.write_text(json.dumps(doc), encoding="utf-8")

            self.assert_failure(
                base, mutate=wrong, needle="does not match GitHub digest"
            )

    def test_missing_local_file_rejected(self):
        with tempfile.TemporaryDirectory() as base:

            def rm(t, m, a):
                (a / PLATFORMS[0]).unlink()

            self.assert_failure(base, mutate=rm, needle="missing")

    def test_local_file_not_regular_rejected(self):
        with tempfile.TemporaryDirectory() as base:

            def dirify(t, m, a):
                (a / PLATFORMS[0]).unlink()
                (a / PLATFORMS[0]).mkdir()

            self.assert_failure(base, mutate=dirify, needle="not a regular file")

    def test_empty_local_file_rejected(self):
        with tempfile.TemporaryDirectory() as base:

            def empty(t, m, a):
                (a / PLATFORMS[0]).write_bytes(b"")

            self.assert_failure(base, mutate=empty, needle="empty")

    def test_zero_byte_file_rejected_http_error_analogue(self):
        # HTTP-error analogue: the old `curl -sL | sha256sum` silently accepted
        # failed downloads and hashed empty input. A zero-byte asset must
        # never produce a manifest.
        with tempfile.TemporaryDirectory() as base:

            def zero(t, m, a):
                (a / PLATFORMS[0]).write_bytes(b"")

            self.assert_failure(base, mutate=zero, needle="empty")

    def test_size_mismatch_rejected(self):
        with tempfile.TemporaryDirectory() as base:

            def grow(t, m, a):
                path = a / PLATFORMS[0]
                path.write_bytes(b"\x00" * (path.stat().st_size + 1))

            self.assert_failure(base, mutate=grow, needle="local size")

    def test_package_order_fixed_regardless_of_metadata_order(self):
        with tempfile.TemporaryDirectory() as base:
            template, metadata, assets = happy_fixture(base)
            doc = json.loads(metadata.read_text(encoding="utf-8"))
            doc["assets"] = list(reversed(doc["assets"]))
            metadata.write_text(json.dumps(doc), encoding="utf-8")
            manifest = bm.build(str(template), VERSION, str(metadata), str(assets))
            self.assertEqual(
                [p["identifier"] for p in manifest["packages"]], expected_urls()
            )

    def test_cli_failure_leaves_no_output(self):
        with tempfile.TemporaryDirectory() as base:
            template, metadata, assets = happy_fixture(base)
            (assets / PLATFORMS[0]).unlink()
            out = Path(base) / "out" / "server.json"
            code = bm.main(
                [
                    "--template",
                    str(template),
                    "--version",
                    VERSION,
                    "--metadata",
                    str(metadata),
                    "--assets",
                    str(assets),
                    "--output",
                    str(out),
                ]
            )
            self.assertEqual(code, 1)
            self.assertFalse(out.exists())


if __name__ == "__main__":
    unittest.main()
