# Persistent capture (`export_log`)

`export_log` writes a connection's event log as JSONL only when the server starts
with persistent capture enabled. This page defines how to enable capture, the
portable filename rules, quotas, atomicity, and failure behavior.

## Disabled by default

Persistent capture is disabled by default. Without it, `export_log` returns the
exact error
`Persistent capture is disabled; start serial-mcp with --capture-dir <absolute-directory>`.
No file work happens. The server does not fall back to the current directory,
OS config directory, or temporary directories.

Enable capture at startup with these options:

| Option | Meaning | Default |
|---|---|---|
| `--capture-dir <absolute-dir>` | Enable capture in an existing absolute directory | disabled |
| `--capture-max-file-bytes <N>` | Per-file quota for a capture JSONL snapshot | 16 MiB (`16777216`) |
| `--capture-max-total-bytes <N>` | Total-byte quota across committed capture files | 256 MiB (`268435456`) |
| `--capture-max-files <N>` | File-count quota across committed capture files | 256 |

The configured root must be an existing directory. It must not itself be a
symlink. The root is canonicalized once at startup. Supplying quota options
without `--capture-dir` is a startup error. Invalid root and quota relationships
are also startup errors.

## Filename-only portable contract

The `export_log` `path` field is a portable `.jsonl` filename relative to the
capture root. It is never an arbitrary path.

- Names use ASCII and contain 1 to 120 characters. The `.jsonl` suffix is case-sensitive.
- Names start with an alphanumeric character.
- Allowed characters are alphanumeric characters, `.`, `_`, and `-`.
- Names may not contain `/` or `\`. They may not be `.` or `..`.
- Names may not use the `.serial-mcp-` reserved prefix.
- Windows-reserved stems include `CON`, `PRN`, `AUX`, `NUL`, `COM1` through
  `COM9`, and `LPT1` through `LPT9`. The check is case-insensitive. It also
  applies when an extension is present.

The namespace is flat by design. The server accepts no separators or
subdirectories. It accepts no traversal or absolute paths. It never accepts a
caller-supplied directory or creates subdirectories.

## Atomic point-in-time snapshots

Each export is a complete point-in-time snapshot of the connection's event log.
It is committed atomically with `persist_noclobber`.

- An existing file, symlink, directory, or special file is rejected.
  `export_log` never overwrites files or follows symlinks.
- The server writes to a same-root temporary file. It syncs that file and
  atomically renames it into place. No final file exists before a successful
  commit.
- A surviving `.serial-mcp-capture-*` temporary file is never treated as
  committed. It is never deleted.

## Quotas and the advisory lock

Per-file, total-byte, and file-count quotas are enforced from a fresh scan of
the root's direct children. The scan runs under the advisory cross-process
lock `.serial-mcp-captures.lock`.

Cooperating serial-mcp processes that share a root cannot exceed the quotas. The
lock file and `.serial-mcp-capture-*` temporary files are reserved. They are
excluded from quota accounting. Unknown and orphan entries are never deleted.

## Failure semantics and counts

- A failure before commit creates no file. It changes no existing capture. The
  connection remains usable.
- Success returns the canonical absolute final path. It also returns the exact
  `bytes_written`, `files_used`, and `total_bytes_used` counts.
- On Unix, the root directory is synced after commit. If that sync fails, the
  export still succeeds and reports a `durability_warning`. The file is
  committed, counted, and never deleted.
- Windows does not attempt a root sync. It documents the rename crash-durability
  limitation instead.

## Trust boundary

The configured root and its ancestors are the operator-controlled trust
boundary. The server does not defend against a root writable by an attacker.
The advisory lock protects cooperating serial-mcp processes only.

## Breaking migration

This contract removes the earlier behavior of writing to an arbitrary
caller-supplied absolute path. Workflows that passed absolute paths to
`export_log` must use the portable filename form instead.
