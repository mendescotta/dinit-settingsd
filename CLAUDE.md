@AGENTS.md

# dinit-settingsd

D-Bus hostname1, timedate1 and locale1 services for Void on dinit.
The runit-settingsd repository is a deliberate
counterpart; review shared behavior in both when changing protocol or file edits.

- Preserve the D-Bus interfaces and the read-only mode.
- Test writes under a scratch `--root` and session bus; do not change host
  hostname, timezone, locale, service links, or system bus during verification.
- Keep dinit NTP service handling specific to this repository.
- Verify with `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  and `cargo test`.
