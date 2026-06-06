# Feature Ideas

## Richer matching modes

- Add optional regex matching for `read` and `subscribe` match config.
- Add optional glob-style matching for `read` and `subscribe` match config.
- Keep current default behavior as literal byte-substring matching for backward compatibility.
- Likely shape: add `match_mode` with values like `substring` (default), `regex`, `glob`.
- Add smarter context slicing around regex/glob matches once richer matching exists.

## Multiple public subscriptions per connection

- Consider allowing more than one public `subscribe` on the same connection.
- Possible use cases:
  - one raw log stream plus one match watcher
  - multiple different match patterns at once
  - long-lived monitor plus short-lived diagnostic stream
- Likely requires:
  - subscription IDs
  - unsubscribe by subscription ID
  - clear budget accounting per subscription
  - defined fanout/duplication semantics
- Simpler alternative to evaluate later:
  - keep one public subscription per connection but allow reconfiguration/update
    semantics instead of full replacement
