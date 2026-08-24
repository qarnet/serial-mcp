#!/usr/bin/env python3
"""Build the MCP Registry ``server.json`` manifest for a released version.

Pure Python standard library, fully offline, fail-closed. Every input is
validated before anything is written; the output file only exists after all
checks pass and is committed atomically (temp file + rename).

Validation failures exit non-zero with a specific message and never leave a
partial or stale output file behind.
"""

import argparse
import hashlib
import json
import os
import re
import sys
import tempfile
from pathlib import Path

REPO = "qarnet/serial-mcp"
EXPECTED_ASSETS = [
    "serial-mcp-x86_64-linux",
    "serial-mcp-aarch64-linux",
    "serial-mcp-aarch64-macos",
    "serial-mcp-x86_64-windows.exe",
]

SEMVER_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")
SHA256_RE = re.compile(r"^sha256:([0-9a-fA-F]{64})$")
BARE_SHA256_RE = re.compile(r"^[0-9a-fA-F]{64}$")


class BuildError(Exception):
    """A validation failure that must abort before any output is written."""


def validate_version(version):
    """Strict SemVer: MAJOR.MINOR.PATCH only (no pre-release/build metadata)."""
    if not isinstance(version, str) or not SEMVER_RE.fullmatch(version):
        raise BuildError(
            f"invalid version {version!r}: expected strict SemVer MAJOR.MINOR.PATCH"
        )


def load_template(template_path):
    """Load the historical template; it must have no packages array and must
    carry a version field (checked against the requested version by the
    caller)."""
    try:
        with open(template_path, encoding="utf-8") as fh:
            doc = json.load(fh)
    except OSError as exc:
        raise BuildError(f"cannot read template {template_path}: {exc}") from exc
    except json.JSONDecodeError as exc:
        raise BuildError(f"template {template_path} is not valid JSON: {exc}") from exc
    if not isinstance(doc, dict):
        raise BuildError(f"template {template_path} is not a JSON object")
    if "packages" in doc:
        raise BuildError(
            f"template {template_path} already contains a packages array; "
            "templates must omit it; packages are generated at publish time"
        )
    if not isinstance(doc.get("version"), str):
        raise BuildError(f"template {template_path} has no string version field")
    return doc


def load_metadata(metadata_path, version):
    """Load release metadata. Shape: {"tag": "vX.Y.Z", "assets":
    [{"name": ..., "size": int, "digest": "sha256:hex"|"hex"}]}.
    The asset list must be exactly the four expected platforms with no
    duplicates or extras, and the tag must match v<version>."""
    try:
        with open(metadata_path, encoding="utf-8") as fh:
            meta = json.load(fh)
    except OSError as exc:
        raise BuildError(
            f"cannot read release metadata {metadata_path}: {exc}"
        ) from exc
    except json.JSONDecodeError as exc:
        raise BuildError(
            f"release metadata {metadata_path} is not valid JSON: {exc}"
        ) from exc
    if not isinstance(meta, dict):
        raise BuildError(f"release metadata {metadata_path} is not a JSON object")
    expected_tag = f"v{version}"
    if meta.get("tag") != expected_tag:
        raise BuildError(
            f"release tag {meta.get('tag')!r} does not match version {version!r} "
            f"(expected {expected_tag!r})"
        )
    assets = meta.get("assets")
    if not isinstance(assets, list):
        raise BuildError(f"release metadata {metadata_path} has no assets list")
    by_name = {}
    for entry in assets:
        if not isinstance(entry, dict):
            raise BuildError(f"release metadata asset {entry!r} is not an object")
        name = entry.get("name")
        if not isinstance(name, str) or not name:
            raise BuildError(f"release metadata asset is missing a name: {entry!r}")
        if name in by_name:
            raise BuildError(f"release metadata has duplicate asset {name!r}")
        by_name[name] = entry
    missing = [name for name in EXPECTED_ASSETS if name not in by_name]
    if missing:
        raise BuildError(
            f"release metadata is missing expected assets: {', '.join(missing)}"
        )
    unexpected = [name for name in by_name if name not in EXPECTED_ASSETS]
    if unexpected:
        raise BuildError(
            f"release metadata has unexpected assets: {', '.join(sorted(unexpected))}"
        )
    parsed = {}
    for name in EXPECTED_ASSETS:
        entry = by_name[name]
        size = entry.get("size")
        if not isinstance(size, int) or size <= 0:
            raise BuildError(f"asset {name!r} has invalid size {size!r}")
        digest = entry.get("digest")
        if not isinstance(digest, str) or not digest:
            raise BuildError(f"asset {name!r} digest {digest!r} is missing")
        if BARE_SHA256_RE.fullmatch(digest):
            raise BuildError(
                f"asset {name!r} digest {digest!r} must carry the explicit "
                "sha256: algorithm prefix (GitHub release assets emit "
                "'sha256:<64 hex chars>')"
            )
        match = SHA256_RE.fullmatch(digest)
        if not match:
            raise BuildError(
                f"asset {name!r} digest {digest!r} is not a sha256 digest "
                "(expected 'sha256:<64 hex chars>')"
            )
        parsed[name] = {"size": size, "sha256": match.group(1).lower()}
    return parsed


def _local_digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def build(template_path, version, metadata_path, assets_dir):
    """Validate everything and return the manifest dict (no file writes)."""
    validate_version(version)
    template = load_template(template_path)
    if template.get("version") != version:
        raise BuildError(
            f"template version {template.get('version')!r} does not match "
            f"requested version {version!r}"
        )
    metadata = load_metadata(metadata_path, version)
    assets_dir = Path(assets_dir)
    packages = []
    for name in EXPECTED_ASSETS:
        path = assets_dir / name
        if not path.is_file():
            raise BuildError(
                f"asset {name!r} missing at {path} (or not a regular file)"
            )
        size = path.stat().st_size
        if size <= 0:
            raise BuildError(f"asset {name!r} at {path} is empty")
        expected = metadata[name]
        if size != expected["size"]:
            raise BuildError(
                f"asset {name!r} local size {size} differs from GitHub metadata "
                f"size {expected['size']}"
            )
        digest = _local_digest(path)
        if digest != expected["sha256"]:
            raise BuildError(
                f"asset {name!r} sha256 {digest} does not match GitHub digest "
                f"{expected['sha256']}"
            )
        url = f"https://github.com/{REPO}/releases/download/v{version}/{name}"
        packages.append(
            {
                "registryType": "mcpb",
                "identifier": url,
                "version": version,
                "fileSha256": digest,
                "transport": {"type": "stdio"},
            }
        )
    manifest = dict(template)
    manifest["packages"] = packages
    return manifest


def write_atomic(manifest, output_path):
    """Serialize and commit atomically (same-directory temp + rename)."""
    output = Path(output_path)
    output.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(
        dir=str(output.parent), prefix=".server.json.", suffix=".tmp"
    )
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            json.dump(manifest, fh, indent=2)
            fh.write("\n")
        os.replace(tmp, output)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def main(argv=None):
    parser = argparse.ArgumentParser(
        description=(
            "Build the MCP Registry server.json manifest for a released "
            "version. Offline, fail-closed: nothing is written unless every "
            "input validates."
        )
    )
    parser.add_argument("--template", help="historical template server.json")
    parser.add_argument("--version", help="strict SemVer version (MAJOR.MINOR.PATCH)")
    parser.add_argument("--metadata", help="release metadata JSON (tag + assets)")
    parser.add_argument("--assets", help="directory with the four release assets")
    parser.add_argument("--output", help="output manifest path")
    parser.add_argument(
        "--validate-version-only",
        metavar="VERSION",
        help="validate VERSION and exit (used before any tag/path argument)",
    )
    args = parser.parse_args(argv)

    if args.validate_version_only is not None:
        try:
            validate_version(args.validate_version_only)
        except BuildError as exc:
            print(f"error: {exc}", file=sys.stderr)
            return 1
        return 0

    required = ["template", "version", "metadata", "assets", "output"]
    missing = [name for name in required if getattr(args, name) is None]
    if missing:
        parser.error(f"missing required arguments: {', '.join(missing)}")
    try:
        manifest = build(args.template, args.version, args.metadata, args.assets)
        write_atomic(manifest, args.output)
    except BuildError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
