# prop

`prop` is a fast DNS propagation CLI built as a single Rust binary. It queries a curated set of public resolvers around the world, compares answers for a record, and can also query the parent path for the target TLD.

## Installation

Install Rust and Cargo with a toolchain that supports Rust edition 2024, then run this command from the project directory:

```bash
cargo install --path . --locked
```

Cargo builds an optimized standalone binary. With the default Cargo configuration on macOS and Linux, it installs `prop` in `~/.cargo/bin`; Rust is only required to build or update it.

Check the installation:

```bash
prop --version
prop --help
```

If your shell reports `prop: command not found`, add Cargo's binary directory to your `PATH`. For zsh, add this line to `~/.zshrc`:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

Then reload the configuration:

```bash
source ~/.zshrc
```

After updating the source, reinstall from the project directory:

```bash
cargo install --path . --locked --force
```

To run directly from the source without installing:

```bash
cargo run -- check example.com
```

## Usage

```bash
prop check example.com
prop check example.com --record-type txt --json
prop check example.com --region asia --provider Google
prop trace-parent example.com
prop servers --all
prop config --validate
```

`prop --help` lists all commands and their options. Use `prop <command> --help` or `prop help <command>` for a specific command.

### General options

| Option | Description |
| --- | --- |
| `--config <CONFIG>` | Read a custom TOML configuration file. Can appear before or after the subcommand. |
| `-h`, `--help` | Show help for the current command. |
| `-V`, `--version` | Show the version with `prop --version`. |

### `check <NAME>`

Check DNS propagation for the required domain name across public resolvers.

| Option | Description |
| --- | --- |
| `--record-type <RECORD_TYPE>` | Record type: `a`, `aaaa`, `cname`, `mx`, `ns`, or `txt`. Default: `a`. |
| `--json` | Print the report as JSON. |
| `--region <REGION>` | Only query resolvers in the specified region. |
| `--provider <PROVIDER>` | Only query resolvers from the specified provider. |
| `--tag <TAG>` | Only query resolvers with the specified tag. |

Region, provider, and tag filters match complete values without regard to case. When combined, all filters must match. Use `prop servers --json` to inspect resolver metadata and available filter values.

### `trace-parent <NAME>`

Query the parent TLD nameservers for the required domain's delegation.

| Option | Description |
| --- | --- |
| `--json` | Print the parent trace as JSON. |

### `servers`

List configured DNS resolvers. By default, only enabled resolvers are shown.

| Option | Description |
| --- | --- |
| `--json` | Print the resolver list as JSON. |
| `--all` | Include disabled resolvers. |

### `config`

Show the configuration file path.

| Option | Description |
| --- | --- |
| `--validate` | Validate the configuration and resolver catalog, printing `config_valid=true` on success. |

For example, validate a custom configuration file:

```bash
prop --config ./example-config.toml config --validate
```

## Config

The default configuration file is optional: when it does not exist, `prop` uses its built-in settings and resolver catalog. A file explicitly selected with `--config` must exist and be readable. Run `prop config` to see the default path on your system.

The file is stored in the platform configuration directory under `prop/config.toml`. On Linux, this is `$XDG_CONFIG_HOME/prop/config.toml`, or `~/.config/prop/config.toml` when `XDG_CONFIG_HOME` is unset. On macOS, it is `~/Library/Application Support/prop/config.toml`.

Use `--config` to select a different path:

```bash
prop --config ./example-config.toml check example.com
```

`settings.output` accepts `"table"` (the default) or `"json"`. The `--json` option also enables JSON output for `check`, `trace-parent`, and `servers`.

`settings.timeout_ms` and `settings.concurrency` must be greater than zero. A `check` command whose filters match no enabled resolvers exits with an error.

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
- `check` distinguishes successful answers (`success` in JSON), missing records (`no_records`), nonexistent domains (`nxdomain`), and query failures (`error`, including DNS `SERVFAIL` and `REFUSED`). Errors are also shown in table output.
- `trace-parent` sends non-recursive queries to the root and TLD authorities and reads delegation nameservers from the authority section, with any associated IPv4/IPv6 glue.
