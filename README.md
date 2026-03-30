# prop

`prop` is a fast DNS propagation CLI built as a single Rust binary. It queries a curated set of public resolvers around the world, compares answers for a record, and can also query the parent path for the target TLD.

## Commands

```bash
cargo run -- check example.com
cargo run -- check example.com --record-type txt --json
cargo run -- trace-parent example.com
cargo run -- servers
cargo run -- config --validate
```

## Config

Default config path:

```text
$XDG_CONFIG_HOME/prop/config.toml
```

If `XDG_CONFIG_HOME` is not available, `prop` falls back to the platform config directory.

Example:

```toml
[settings]
timeout_ms = 2500
concurrency = 16
output = "table"

[[servers]]
id = "custom-singapore"
name = "Custom Singapore"
address = "203.0.113.10"
region = "asia"
country = "SG"
provider = "Example"
enabled = true
tags = ["apac", "custom"]

[[overrides]]
id = "google-primary"
enabled = false
tags = ["disabled"]
```

## Notes

- `check` defaults to `A` records.
- Supported record types in v1: `A`, `AAAA`, `CNAME`, `MX`, `NS`, `TXT`.
- Queries run over UDP with TCP fallback when a response is truncated.
