# prop

`prop` is a fast DNS propagation CLI built as a single Rust binary. It queries a curated set of public resolvers around the world, compares answers for a record, and can also query the parent path for the target TLD.

## Installation

### Prebuilt releases

Install prop 0.1.3 using the installer for your system.

#### macOS and Linux

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/mcdado/prop/releases/download/v0.1.3/prop-installer.sh | sh
```

#### Windows (PowerShell)

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/mcdado/prop/releases/download/v0.1.3/prop-installer.ps1 | iex"
```

For manual installation, download the archive for your platform from [GitHub Releases](https://github.com/mcdado/prop/releases), verify its SHA-256 checksum, extract it, and put `prop` (or `prop.exe` on Windows) in a directory on your `PATH`.

Release builds cover macOS Apple Silicon and Intel, Linux ARM64 and x86-64 (GNU), Linux x86-64 (musl), and Windows x86-64.

### Build from source

Install Rust 1.88 or newer and Cargo, then run this command from the project directory:

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

### Shell completions

`prop completions <SHELL>` prints a completion script for `bash`, `zsh`, `fish`, or `powershell`. Completions include subcommands and option names, plus `--record-type` values in Bash, Zsh, and Fish.

#### Bash

Run this in Bash to activate completions for the current session. Add the same line to your Bash startup file (usually `~/.bashrc`) to enable them in future sessions:

```bash
eval "$(prop completions bash)"
```

#### Zsh

Add the following to `~/.zshrc`, after any existing completion-system initialization. If your configuration already calls `compinit`, add only the `source` line:

```zsh
autoload -Uz compinit
compinit
source <(prop completions zsh)
```

Open a new Zsh session, or run these lines in the current one.

#### Fish

Run this in Fish, and add it to `~/.config/fish/config.fish` to enable completions in future sessions:

```fish
prop completions fish | source
```

#### PowerShell

Run this in PowerShell 7 or newer, and add the same line to your PowerShell profile (`$PROFILE`) to enable completions in future sessions:

```powershell
prop completions powershell | Out-String | Invoke-Expression
```

In PowerShell, complete options before entering the positional domain name, for example `prop check --rec<Tab>`.

These startup commands generate completions from the installed `prop`, so they pick up CLI changes after an upgrade.

## Usage

```bash
prop check example.com
prop check example.com --record-type txt --json
prop check example.com --region asia --provider Google
prop check example.com --record-type ns --timeout-ms 10000
prop trace-parent example.com
prop servers --all
prop config --validate
prop completions zsh
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
| `--timeout-ms <TIMEOUT_MS>` | Per-query timeout in milliseconds. Overrides `settings.timeout_ms`; built-in default: `3000`. Must be greater than zero. |
| `--json` | Print the report as JSON. |
| `--region <REGION>` | Only query resolvers in the specified region. |
| `--provider <PROVIDER>` | Only query resolvers from the specified provider. |
| `--tag <TAG>` | Only query resolvers with the specified tag. |

Region, provider, and tag filters match complete values without regard to case. When combined, all filters must match. Use `prop servers --json` to inspect resolver metadata and available filter values.

The built-in catalog uses geographic regions such as `europe`, `asia`, and `north-america`, with `global` for resolvers marked as global. Tables display the region and ISO country code together, for example `europe/IT` or `global/US`. To select a country, use its tag, for example `--region europe --tag italy`. Country names (`italy`, `france`, `spain`, `san-marino`) are no longer region values; their existing tags remain available. Custom resolver metadata and overrides keep the values configured in TOML.

In table output, the query and consensus summary appears below the resolver results.

Consensus compares complete normalized answer sets and requires a strict majority of valid DNS responses. Timeouts and DNS errors do not vote; empty responses and `NXDOMAIN` vote separately. A tie or any other result without a strict majority produces no consensus, rather than combining records from different resolvers.

The JSON `consensus` field remains an array of answers. Additional fields explain the result: `consensus_status` (`unanimous`, `majority`, `no_majority`, or `no_responses`), `consensus_response_status`, `agreeing_resolvers`, and `responding_resolvers`. A single valid response can therefore be unanimous even when other resolvers fail; inspect the per-resolver results for those failures.

DNS names in CNAME, NS, and MX answers are compared without regard to case. Chunks within each TXT record are concatenated without adding spaces; separate TXT records remain separate answers.

### `trace-parent <NAME>`

Query the parent TLD nameservers for the required domain's delegation.

| Option | Description |
| --- | --- |
| `--json` | Print the parent trace as JSON. |
| `--timeout-ms <TIMEOUT_MS>` | Per-query timeout in milliseconds. Overrides `settings.timeout_ms`; built-in default: `3000`. Must be greater than zero. |

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

The built-in query timeout is 3000 ms. Set `settings.timeout_ms` to change it persistently, or pass `--timeout-ms` to `check` or `trace-parent` for a single run. The command-line value takes precedence over the configuration file. This limit applies to each query, rather than to the entire command.

Example:

```toml
[settings]
timeout_ms = 3000
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

## Development and releases

CI runs tests on Linux, macOS, and Windows, plus formatting, Clippy, and release configuration checks. The same checks gate release builds.

Distribution is configured in `dist-workspace.toml` using cargo-dist **0.33.0**. To install that development tool, run `cargo install cargo-dist --version 0.33.0 --locked`.

After changing the distribution configuration or upgrading cargo-dist, regenerate and verify the workflow with:

```bash
node scripts/generate-release.mjs
node scripts/generate-release.mjs --check
dist plan
```

Use this script instead of `dist generate` for CI generation. It applies the project's draft-only policy to cargo-dist's generated workflow; CI checks that this adaptation is up to date. Existing published releases are never modified by the workflow, and reruns can update an existing draft.

To prepare a release:

1. Update the package version in `Cargo.toml` and `Cargo.lock`, and add release notes to `CHANGELOG.md`.
2. Commit and push to `main`, and wait for CI to pass.
3. Create and push an annotated tag matching the version, for example:

   ```bash
   git tag -a v0.1.3 -m "prop 0.1.3"
   git push origin v0.1.3
   ```

4. Wait for the Release workflow to build the archives, installers, source package, and checksums and attach them to a **draft** GitHub Release.
5. Review the release notes and downloads, then publish the draft explicitly from GitHub when ready.
