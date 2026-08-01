# Code Cleanup Phase 1 Fix Handoff

## Review verdict

Behavior and verification are good, but Phase 1 is not accepted yet. Shared TX
preparation became more elaborate than duplicated code it replaced:

- `DecodedTxPayload` wraps one `Vec<u8>` plus values callers already own or can
  derive.
- `PreparedTxData` wraps `Arc<[u8]>` plus the same encoding/length metadata.
- Production helper commentary is longer than logic.

Cleanup goal is concise shared buffer transformation, not a new internal model.

## Required fix

Edit `src/tools/io_ops.rs` only, plus this handoff document.

1. Replace `DecodedTxPayload` with:

```rust
fn decode_tx_payload(
    encoding: Encoding,
    input: &str,
    decoded_limit_field: &str,
) -> Result<Vec<u8>, String>
```

It must preserve current decode error and decoded-size validation behavior.

2. Replace `PreparedTxData` with a framing helper returning only prepared bytes:

```rust
fn apply_tx_framing(
    decoded: Vec<u8>,
    framing: Option<&crate::framing::TxFramingConfig>,
    framed_limit_field: &str,
) -> Result<Arc<[u8]>, TxFramingError>
```

Equivalent concise naming is fine. Keep the small `TxFramingError` distinction
because callers require different framing error mapping while size errors pass
through unchanged.

3. In `write` and `transact`:

- Compute `decoded_len` from the decoded vector before moving it into framing.
- Keep using the already parsed local `encoding` for result metadata.
- Pass returned `Arc<[u8]>` directly to session write.
- Preserve exact ordering and errors from Phase 1 handoff.

4. Tighten helper comments. One short statement per helper/error type is enough.
   Remove comments that restate fields or implementation syntax.

5. Update private helper tests to assert meaningful bytes/errors with new return
   types. Keep representative UTF-8/hex/base64 decode, unframed bytes, framed
   bytes, and framing-vs-size error behavior. Remove assertions for deleted
   wrapper metadata.

## Non-scope

- Do not alter post-read accounting helper.
- Do not change public behavior, schemas, error text, or operation ordering.
- Do not perform Phase 2 or unrelated comment cleanup.
- Do not amend the prior commit.

## Verification

```bash
cargo fmt --all -- --check
cargo test --lib tools::io_ops
cargo test --test serial_pty pty_transact --locked -- --test-threads=1
cargo clippy --all-targets --locked -- -D warnings
```

Inspect status/diff/log, stage only `src/tools/io_ops.rs` and this fix handoff,
then create a new commit:

`refactor: simplify TX preparation helpers`

Do not push, merge, open a PR, amend, force-push, or add attribution. Return
files, behavior, checks, commit hash/message, deviations, and blockers.
