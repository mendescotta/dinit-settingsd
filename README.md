# dinit-settingsd

`org.freedesktop.hostname1`, `timedate1` and `locale1` D-Bus services for Void Linux on dinit
(the [dinit-void](https://github.com/mendescotta/voidlab) stack), so GNOME, Cinnamon, KDE and COSMIC
settings panels can change the hostname, timezone, NTP, RTC mode, locale and keyboard layout without
systemd. One bus-activated daemon that exits after 60 s idle, plus `timedatectl`, `hostnamectl` and
`localectl` wrappers.

The dinit counterpart of [runit-settingsd](https://github.com/mendescotta/runit-settingsd): same
interfaces, different init handling. Both own the same D-Bus names, so install one.

## Install

From the voidlab binary repository (x86_64 glibc):

```sh
printf 'repository=https://github.com/mendescotta/voidlab/releases/download/repo\nbestmatching=true\n' |
    sudo tee /etc/xbps.d/20-voidlab.conf
sudo xbps-install -S dinit-settingsd    # accept the voidlab signing key
```

From source (needs `rust`, `cargo`; runtime `dbus`, `polkit`, `tzdata`, `dinit`):

```sh
cargo build --release
sudo install -Dm755 target/release/dinit-settingsd /usr/libexec/dinit-settingsd
sudo install -Dm755 target/release/dinit-settingsctl /usr/bin/dinit-settingsctl
sudo install -Dm644 man/dinit-settingsd.8 /usr/share/man/man8/dinit-settingsd.8
sudo install -Dm644 man/dinit-settingsctl.1 /usr/share/man/man1/dinit-settingsctl.1
for t in timedatectl hostnamectl localectl; do sudo ln -sf dinit-settingsctl /usr/bin/$t; done
sudo install -Dm644 data/org.freedesktop.settingsd.conf /usr/share/dbus-1/system.d/org.freedesktop.settingsd.conf
sudo install -Dm644 data/org.freedesktop.settingsd.policy /usr/share/polkit-1/actions/org.freedesktop.settingsd.policy
for n in hostname1 timedate1 locale1; do
    sudo install -Dm644 data/org.freedesktop.$n.service /usr/share/dbus-1/system-services/org.freedesktop.$n.service
done
```

Nothing to enable: the system bus starts the daemon when a settings panel (or `timedatectl` etc.) first
talks to it. Requires a running `dbus` and `polkit`.

## What it writes

`/etc/hostname`, `/etc/machine-info`, `/etc/localtime`, `/etc/adjtime`, `/etc/locale.conf`,
`/etc/rc.conf` (KEYMAP, TIMEZONE if set) and `/etc/X11/xorg.conf.d/*keyboard.conf`; dinit-void's early
boot reads the hostname and `rc.conf` keymap from these.

NTP is a dinit service: enabling links `/etc/dinit.d/<svc>` into `/etc/dinit.d/boot.d/` and runs
`dinitctl --system start <svc>` so it takes effect now; disabling unlinks it and stops it. The link is
what persists, so a `dinitctl` failure is reported on stderr but does not undo it. `ntp-services=` in
`/etc/dinit-settingsd.conf` sets the order (default `chronyd`).

## timedatectl / hostnamectl / localectl

`dinit-settingsctl` is a multi-call client; run it through the symlinks above or as
`dinit-settingsctl <tool> ...`.

| Tool | Commands |
|---|---|
| `timedatectl` | status, show, set-timezone, list-timezones, set-ntp, set-local-rtc |
| `hostnamectl` | status, hostname, set-hostname (`--static/--pretty/--transient`), set-icon-name, set-chassis, set-deployment, set-location |
| `localectl` | status, list-locales, set-locale, set-keymap, set-x11-keymap |

These are not the full systemd CLIs (no `set-time`). Options: `--read-only` refuses all changes,
`--ntp-service NAME` uses only that dinit service for NTP, `--version`. Man pages:
`dinit-settingsd(8)`, `dinit-settingsctl(1)`.

## Development

```sh
cargo test
dbus-run-session -- target/debug/dinit-settingsd --bus session --root DIR --no-auth --persist
```

`--root` points the daemon at a scratch tree (session bus only), which is how the tests run.

## References

Design cross-checked against postmarketOS's
[openrc-settingsd](https://gitlab.postmarketos.org/postmarketOS/openrc-settingsd); this is an independent
Rust implementation, derived from runit-settingsd.

Licensed under GPL-3.0-or-later.
