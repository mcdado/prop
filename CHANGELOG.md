# Changelog

## 0.1.3 - 2026-10-20

- Add `prop completions <SHELL>` for Bash, Zsh, Fish, and PowerShell, with activation instructions in the README.
- Generate completions even when the configuration file is missing or invalid.
- Use Bash activation compatible with the Bash 3 version included in macOS, and verify activation in all four shells in CI.

## 0.1.2 - 2026-10-20

### Security

- Migrate to `hickory-net` and `hickory-proto` 0.26.3 to fix the DNS message encoding CPU exhaustion advisory (RUSTSEC-2026-0119).
- Replace the vulnerable `rand` 0.8.5 dependency with 0.10.3 (RUSTSEC-2026-0097).
- Update `anyhow` to 1.0.103 to fix the `Error::downcast_mut()` soundness advisory (RUSTSEC-2026-0190).

### Fixes

- Preserve recursion flags, EDNS, negative responses, and the configured query timeout when using the new UDP and TCP clients.
- Increase mock network exchange timeouts for shared Windows CI runners and report client failures before waiting for the mock server. CI repeats the affected regression test ten times on Windows.

Building from source now requires Rust 1.88 or newer.

## 0.1.1 - 2026-10-20

First public release of `prop`, a standalone DNS propagation CLI.

### Features

- Compare A, AAAA, CNAME, MX, NS, and TXT records across a built-in public resolver catalog.
- Filter resolvers by region, provider, or tag, and customize the catalog through TOML.
- Trace parent-zone delegations through root and TLD nameservers.
- Show tables or structured JSON, including query timing, TTLs, consensus, and errors.
- Download prebuilt binaries for macOS, Linux, and Windows, with shell and PowerShell installers and SHA-256 checksums.

### Fixes

- Read parent delegation nameservers from authority responses and send non-recursive queries to authorities.
- Distinguish NXDOMAIN, missing records, and DNS failures such as SERVFAIL and REFUSED.
- Preserve query options when falling back from truncated UDP replies to TCP.
- Reject zero timeout/concurrency settings, missing explicit configuration files, and selections without enabled resolvers.
- Compute consensus from complete responses with a strict majority, keeping negative answers distinct.
- Reassemble TXT chunks without artificial spaces and normalize hostname case in CNAME, NS, and MX answers.
