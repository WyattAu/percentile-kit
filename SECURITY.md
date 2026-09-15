# Security Policy — percentile-kit

## Supported versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | ✅        |

## Reporting a vulnerability

Report privately via [GitHub security advisories] for this repository, or
email **wyatt_au@protonmail.com**. Do **not** open a public issue for
security reports.

You will receive an acknowledgement within **72 hours**. Coordinated
disclosure: we ask for up to 90 days before public disclosure while a
patch ships.

## Scope notes

`percentile-kit` tracks in-process latencies and parses criterion output.
Security considerations for integrators:

- **Budget files are trusted input.** `percentile-budgets.toml` gates CI
  and should be committed and reviewed; a malicious budgets file can only
  loosen *your* gates, but do not point `check_budgets` at untrusted
  paths.
- **Criterion reports are parsed tolerantly but schema-checked.**
  `estimates.json` / `sample.json` are deserialized with serde (no
  unsafe, no allocation from untrusted strings beyond the parsed values).
  Point `check_budgets` only at your own `target/criterion` output —
  crafted reports could understate latencies and silently pass budgets.
- The runtime tracker performs **no allocations on the hot path** and
  rejects NaN samples at append; a snapshot only ever contains exact,
  previously recorded values.
- `#![forbid(unsafe_code)]` — no unsafe blocks exist in this crate.

[GitHub security advisories]:
    https://github.com/WyattAu/percentile-kit/security/advisories/new
