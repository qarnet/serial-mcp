# Persistent Capture (`export_log`)

`export_log` persists a connection's event log as JSONL — but only when the
server starts with persistent capture enabled. This is the canonical contract
for enabling it, the portable filename rules, quotas, atomicity, and failure
semantics.

## Disabled by default

Persistent capture is **disabled by default**. Without it, `export_log` errors
with the exact message
`Persistent capture is disabled; start serial-mcp with --capture-dir <absolute-directory>`
and no file work happens — there is no fallback to the current directory, OS
config, or temp dirs.

Enable it at startup with the capture options:

| Option | Meaning | Default |
|---|---|---|
| `--capture-dir <absolute-dir>` | Enable capture into an **existing absolute directory** | disabled |
| `--capture-max-file-bytes <N>` | Per-file quota for a capture JSONL snapshot | 16 MiB (`16777216`) |
| `--capture-max-total-bytes <N>` | Total-byte quota across committed capture files | 256 MiB (`268435456`) |
| `--capture-max-files <N>` | File-count quota across committed capture files | 256 |

The configured root must be an **existing directory** (not itself a symlink)
and is canonicalized once at startup. Quota options supplied without
`--capture-dir` are startup errors; invalid root/quota relations are startup
errors.

## Filename-only portable contract

`export_log`'s `path` field is a **portable `.jsonl` filename relative to the
capture root** — never an arbitrary path:

- ASCII, 1–120 characters, ending `.jsonl` (case-sensitive suffix);
- starts alphanumeric; only alphanumeric/`.`/`_`/`-` allowed;
- no `/` or `\`, not `.` or `..`;
- no `.serial-mcp-` reserved prefix;
- no Windows-reserved stems (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`,
  `LPT1`–`LPT9`), rejected case-insensitively including with an extension.

No separators, no subdirectories, no traversal, no absolute paths — a **flat
namespace** by design: the server never creates subdirectories and never
accepts a caller-supplied directory.

## Atomic point-in-time snapshots

Every export is a complete point-in-time snapshot of the connection's event
log, committed atomically with `persist_noclobber`:

- an existing file (regular, symlink, directory, or special) is rejected —
  `export_log` **never overwrites** and never follows symlinks;
- the file is written to a same-root temp file, synced, then atomically
  renamed into place — no final file exists before a successful commit;
- a surviving temp file (`.serial-mcp-capture-*`) is never treated as
  committed and never deleted.

## Quotas and the advisory lock

Per-file, total-byte, and file-count quotas are enforced from a **fresh scan
of the root's direct children** under an **advisory cross-process lock**
(`.serial-mcp-captures.lock`): cooperating serial-mcp processes sharing a root
cannot exceed the quotas. Internal entries (the lock file and
`.serial-mcp-capture-*` temp files) are reserved and excluded from quota
accounting; unknown or orphan entries are never deleted.

## Failure semantics and counts

- A failure **before** the commit creates no file and changes no existing
  capture; the connection stays usable.
- Success returns the canonical absolute final path plus exact counts:
  `bytes_written`, `files_used`, `total_bytes_used`.
- On Unix the root directory is synced after the commit; if that sync fails,
  the export still succeeds but reports a `durability_warning` — the file is
  committed and counted and is never deleted. Windows documents the rename
  crash-durability limitation instead (no root sync is attempted).

## Trust boundary

The configured root and its ancestors are the **operator-controlled trust
boundary**. There is no hostile-root capability defense: a root writable by an
attacker is not defended against. The advisory lock protects cooperating
serial-mcp processes only.

## Breaking migration

This contract **removed** the earlier behavior of writing to an arbitrary
caller-supplied absolute path. Update any workflow that passed absolute paths
to `export_log` — only the portable filename form is accepted now.
