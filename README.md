# jaytripper

JayTripper is a local wormhole navigation assistant for EVE Online.

## Quickstart

Bootstrap and verify:

```bash
make fmt-check
make lint
make test
```

## Commands

- `make fmt` - format Rust sources with nightly rustfmt
- `make fix` - run automated cargo & clippy `--fix` operations
- `make lint` - run clippy for all targets/features with warnings denied
- `make test` - run all workspace tests
