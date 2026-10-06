use crate::auth::{check, io_err, Ctx};
use crate::files;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::{fdo, interface, Connection};

pub struct Timedate(pub Arc<Ctx>);

const NTP_DEFAULT: &[&str] = &["chronyd"];
const ZONEINFO: &str = "/usr/share/zoneinfo";

static NTP_OVERRIDE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn set_ntp_override(service: String) {
    let _ = NTP_OVERRIDE.set(service);
}

fn config(root: &Path) -> String {
    files::read(&files::path(root, "/etc/dinit-settingsd.conf"))
}

fn ntp_candidates(root: &Path) -> Vec<String> {
    if let Some(svc) = NTP_OVERRIDE.get() {
        return vec![svc.clone()];
    }
    let conf = config(root);
    for l in conf.lines() {
        if let Some(v) = l.trim().strip_prefix("ntp-services=") {
            return v
                .split([',', ' '])
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
        }
    }
    NTP_DEFAULT.iter().map(|s| s.to_string()).collect()
}

pub fn installed_ntp(root: &Path) -> Option<String> {
    ntp_candidates(root)
        .into_iter()
        .find(|s| files::path(root, &format!("/etc/dinit.d/{s}")).exists())
}

/// An NTP service is enabled when it is linked into `/etc/dinit.d/boot.d`, the directory the `boot`
/// service waits for.
pub fn active_ntp(root: &Path) -> Option<String> {
    ntp_candidates(root).into_iter().find(|s| {
        files::path(root, &format!("/etc/dinit.d/boot.d/{s}"))
            .symlink_metadata()
            .is_ok()
    })
}

/// Run `dinitctl --system <action> <service>`; on a real system only (a test root has no dinit).
fn dinitctl(root: &Path, action: &str, svc: &str) -> std::io::Result<()> {
    if root != Path::new("/") {
        return Ok(());
    }
    let mut cmd = Command::new("dinitctl");
    cmd.arg("--system").arg(action);
    if action == "stop" {
        cmd.arg("--ignore-unstarted");
    }
    let status = cmd.arg(svc).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "dinitctl {action} {svc} failed ({status})"
        )))
    }
}

pub fn set_ntp(root: &Path, on: bool) -> std::io::Result<()> {
    set_ntp_with(root, on, &|action, svc| dinitctl(root, action, svc))
}

/// Enable or disable NTP: link (or unlink) the service in `boot.d`, then start (or stop) it now so the
/// change takes effect without a reboot. The link is what persists; a dinitctl failure is reported on
/// stderr but does not undo it.
pub fn set_ntp_with(
    root: &Path,
    on: bool,
    ctl: &dyn Fn(&str, &str) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let note = |r: std::io::Result<()>| {
        if let Err(e) = r {
            eprintln!("dinit-settingsd: {e}");
        }
    };
    if on {
        let svc =
            installed_ntp(root).ok_or_else(|| std::io::Error::other("no NTP service installed"))?;
        let link = files::path(root, &format!("/etc/dinit.d/boot.d/{svc}"));
        if link.symlink_metadata().is_err() {
            if let Some(dir) = link.parent() {
                std::fs::create_dir_all(dir)?;
            }
            symlink(format!("/etc/dinit.d/{svc}"), &link)?;
        }
        note(ctl("start", &svc));
    } else {
        for c in ntp_candidates(root) {
            let link = files::path(root, &format!("/etc/dinit.d/boot.d/{c}"));
            if link.symlink_metadata().is_ok() {
                std::fs::remove_file(link)?;
                note(ctl("stop", &c));
            }
        }
    }
    Ok(())
}

pub fn timezone(root: &Path) -> String {
    std::fs::read_link(files::path(root, "/etc/localtime"))
        .ok()
        .and_then(|t| {
            let t = t.to_string_lossy().into_owned();
            t.split_once("zoneinfo/").map(|(_, z)| z.to_string())
        })
        .unwrap_or_else(|| "UTC".into())
}

pub fn list_timezones(root: &Path) -> Vec<String> {
    let zi = files::read(&files::path(root, &format!("{ZONEINFO}/tzdata.zi")));
    let mut v: Vec<String> = zi
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            match it.next()? {
                "Z" => it.next().map(String::from),
                "L" => it.nth(1).map(String::from),
                _ => None,
            }
        })
        .filter(|z| z.contains('/') || z == "UTC")
        .collect();
    v.sort();
    v.dedup();
    v
}

pub fn set_timezone(root: &Path, tz: &str) -> Result<(), String> {
    if tz.is_empty()
        || tz.starts_with('/')
        || tz.split('/').any(|c| c.is_empty() || c == "." || c == "..")
    {
        return Err(format!("Invalid time zone '{tz}'"));
    }
    let target = files::path(root, &format!("{ZONEINFO}/{tz}"));
    if !target.is_file() {
        return Err(format!("Invalid time zone '{tz}'"));
    }
    let link = files::path(root, "/etc/localtime");
    let tmp = files::path(
        root,
        &format!("/etc/.localtime.settingsd.{}", std::process::id()),
    );
    let _ = std::fs::remove_file(&tmp);
    symlink(format!("{ZONEINFO}/{tz}"), &tmp).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &link).map_err(|e| e.to_string())?;
    let rc = files::path(root, "/etc/rc.conf");
    if files::env_get(&rc, "TIMEZONE").is_some() {
        files::env_set(&rc, "TIMEZONE", Some(tz)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn local_rtc(root: &Path) -> bool {
    files::read(&files::path(root, "/etc/adjtime"))
        .lines()
        .nth(2)
        .map(|l| l.trim().eq_ignore_ascii_case("LOCAL"))
        .unwrap_or(false)
}

pub fn set_local_rtc(root: &Path, local: bool) -> std::io::Result<()> {
    let p = files::path(root, "/etc/adjtime");
    let old = files::read(&p);
    let mut lines: Vec<String> = old.lines().map(String::from).collect();
    while lines.len() < 3 {
        lines.push(match lines.len() {
            0 => "0.0 0 0.0".into(),
            1 => "0".into(),
            _ => "UTC".into(),
        });
    }
    lines[2] = if local { "LOCAL".into() } else { "UTC".into() };
    files::atomic_write(&p, &(lines.join("\n") + "\n"))
}

fn rtc_usec(root: &Path) -> u64 {
    files::read(&files::path(root, "/sys/class/rtc/rtc0/since_epoch"))
        .trim()
        .parse::<u64>()
        .map(|s| s * 1_000_000)
        .unwrap_or(0)
}

fn ntp_synchronized() -> bool {
    let mut t: libc::timex = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::adjtimex(&mut t) };
    rc >= 0 && rc != libc::TIME_ERROR && (t.status & libc::STA_UNSYNC) == 0
}

fn run(cmd: &str, args: &[&str]) {
    let _ = Command::new(cmd).args(args).status();
}

#[interface(name = "org.freedesktop.timedate1")]
impl Timedate {
    #[zbus(property)]
    fn timezone(&self) -> String {
        self.0.touch();
        timezone(&self.0.root)
    }
    #[zbus(property, name = "LocalRTC")]
    fn local_rtc(&self) -> bool {
        local_rtc(&self.0.root)
    }
    #[zbus(property, name = "CanNTP")]
    fn can_ntp(&self) -> bool {
        installed_ntp(&self.0.root).is_some()
    }
    #[zbus(property, name = "NTP")]
    fn ntp(&self) -> bool {
        active_ntp(&self.0.root).is_some()
    }
    #[zbus(property, name = "NTPSynchronized")]
    fn ntp_synchronized(&self) -> bool {
        ntp_synchronized()
    }
    #[zbus(property, name = "TimeUSec")]
    fn time_usec(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0)
    }
    #[zbus(property, name = "RTCTimeUSec")]
    fn rtc_time_usec(&self) -> u64 {
        rtc_usec(&self.0.root)
    }

    async fn list_timezones(&self) -> Vec<String> {
        self.0.touch();
        list_timezones(&self.0.root)
    }

    async fn set_timezone(
        &self,
        timezone: String,
        interactive: bool,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] em: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        check(
            &self.0,
            conn,
            &hdr,
            "org.freedesktop.timedate1.set-timezone",
            interactive,
        )
        .await?;
        set_timezone(&self.0.root, &timezone).map_err(fdo::Error::InvalidArgs)?;
        self.timezone_changed(&em).await?;
        Ok(())
    }

    #[zbus(name = "SetLocalRTC")]
    async fn set_local_rtc(
        &self,
        local_rtc: bool,
        fix_system: bool,
        interactive: bool,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] em: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        check(
            &self.0,
            conn,
            &hdr,
            "org.freedesktop.timedate1.set-local-rtc",
            interactive,
        )
        .await?;
        set_local_rtc(&self.0.root, local_rtc).map_err(io_err)?;
        if !self.0.test_mode {
            if fix_system {
                run("hwclock", &["--systz"]);
            } else {
                run("hwclock", &["--systohc"]);
            }
        }
        self.local_r_t_c_changed(&em).await?;
        Ok(())
    }

    #[zbus(name = "SetNTP")]
    async fn set_ntp(
        &self,
        use_ntp: bool,
        interactive: bool,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] em: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        check(
            &self.0,
            conn,
            &hdr,
            "org.freedesktop.timedate1.set-ntp",
            interactive,
        )
        .await?;
        set_ntp(&self.0.root, use_ntp).map_err(|e| fdo::Error::Failed(e.to_string()))?;
        self.n_t_p_changed(&em).await?;
        Ok(())
    }

    async fn set_time(
        &self,
        usec_utc: i64,
        relative: bool,
        interactive: bool,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] em: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        check(
            &self.0,
            conn,
            &hdr,
            "org.freedesktop.timedate1.set-time",
            interactive,
        )
        .await?;
        if active_ntp(&self.0.root).is_some() {
            return Err(fdo::Error::AccessDenied(
                "Automatic time synchronization is enabled".into(),
            ));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as i64)
            .unwrap_or(0);
        let target = if relative { now + usec_utc } else { usec_utc };
        if target < 0 {
            return Err(fdo::Error::InvalidArgs("Invalid time".into()));
        }
        if !self.0.test_mode {
            let ts = libc::timespec {
                tv_sec: target / 1_000_000,
                tv_nsec: (target % 1_000_000) * 1000,
            };
            if unsafe { libc::clock_settime(libc::CLOCK_REALTIME, &ts) } != 0 {
                return Err(io_err(std::io::Error::last_os_error()));
            }
            run("hwclock", &["--systohc"]);
        }
        self.time_u_sec_changed(&em).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn root() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let zi = d.path().join("usr/share/zoneinfo");
        fs::create_dir_all(zi.join("Europe")).unwrap();
        fs::write(zi.join("Europe/Tallinn"), "").unwrap();
        fs::write(zi.join("UTC"), "").unwrap();
        fs::write(
            zi.join("tzdata.zi"),
            "# version\nZ Europe/Tallinn 1:39 - LMT\nZ UTC 0 - UTC\nL Europe/Tallinn Europe/Helsinki_x\nR foo\n",
        )
        .unwrap();
        fs::create_dir_all(d.path().join("etc/dinit.d/boot.d")).unwrap();
        fs::write(d.path().join("etc/dinit.d/chronyd"), "type = process\n").unwrap();
        d
    }

    #[test]
    fn tz_roundtrip_and_validation() {
        let d = root();
        fs::write(d.path().join("etc/rc.conf"), "TIMEZONE=UTC\n").unwrap();
        assert_eq!(timezone(d.path()), "UTC");
        set_timezone(d.path(), "Europe/Tallinn").unwrap();
        assert_eq!(timezone(d.path()), "Europe/Tallinn");
        assert_eq!(
            files::env_get(&d.path().join("etc/rc.conf"), "TIMEZONE").unwrap(),
            "Europe/Tallinn"
        );
        assert!(set_timezone(d.path(), "../etc/passwd").is_err());
        assert!(set_timezone(d.path(), "Nope/Zone").is_err());
        assert!(set_timezone(d.path(), "/UTC").is_err());
    }

    #[test]
    fn tz_list() {
        let d = root();
        assert_eq!(
            list_timezones(d.path()),
            vec!["Europe/Helsinki_x", "Europe/Tallinn", "UTC"]
        );
    }

    #[test]
    fn rtc_mode() {
        let d = root();
        assert!(!local_rtc(d.path()));
        set_local_rtc(d.path(), true).unwrap();
        assert!(local_rtc(d.path()));
        assert_eq!(
            fs::read_to_string(d.path().join("etc/adjtime")).unwrap(),
            "0.0 0 0.0\n0\nLOCAL\n"
        );
        set_local_rtc(d.path(), false).unwrap();
        assert!(!local_rtc(d.path()));
    }

    use std::cell::RefCell;

    /// Record the dinitctl calls instead of running them.
    fn recording(calls: &RefCell<Vec<String>>) -> impl Fn(&str, &str) -> std::io::Result<()> + '_ {
        move |action, svc| {
            calls.borrow_mut().push(format!("{action} {svc}"));
            Ok(())
        }
    }

    #[test]
    fn ntp_toggle_links_into_boot_d_and_drives_dinitctl() {
        let d = root();
        let calls = RefCell::new(Vec::new());
        assert_eq!(installed_ntp(d.path()).as_deref(), Some("chronyd"));
        assert!(active_ntp(d.path()).is_none());

        set_ntp_with(d.path(), true, &recording(&calls)).unwrap();
        assert_eq!(active_ntp(d.path()).as_deref(), Some("chronyd"));
        assert_eq!(
            fs::read_link(d.path().join("etc/dinit.d/boot.d/chronyd")).unwrap(),
            std::path::PathBuf::from("/etc/dinit.d/chronyd")
        );
        assert_eq!(*calls.borrow(), vec!["start chronyd"]);

        set_ntp_with(d.path(), false, &recording(&calls)).unwrap();
        assert!(active_ntp(d.path()).is_none());
        assert!(d
            .path()
            .join("etc/dinit.d/boot.d/chronyd")
            .symlink_metadata()
            .is_err());
        assert_eq!(*calls.borrow(), vec!["start chronyd", "stop chronyd"]);
    }

    #[test]
    fn enabling_twice_is_idempotent_and_disabling_when_off_does_nothing() {
        let d = root();
        let calls = RefCell::new(Vec::new());
        set_ntp_with(d.path(), false, &recording(&calls)).unwrap();
        assert!(calls.borrow().is_empty(), "nothing active: nothing to stop");
        set_ntp_with(d.path(), true, &recording(&calls)).unwrap();
        set_ntp_with(d.path(), true, &recording(&calls)).unwrap();
        assert_eq!(active_ntp(d.path()).as_deref(), Some("chronyd"));
    }

    #[test]
    fn enabling_without_an_ntp_service_installed_is_an_error() {
        let d = root();
        fs::remove_file(d.path().join("etc/dinit.d/chronyd")).unwrap();
        let calls = RefCell::new(Vec::new());
        assert!(set_ntp_with(d.path(), true, &recording(&calls)).is_err());
        assert!(calls.borrow().is_empty());
    }

    #[test]
    fn a_failing_dinitctl_does_not_undo_the_enablement() {
        let d = root();
        let fail = |_: &str, _: &str| Err(std::io::Error::other("dinitctl: connection refused"));
        set_ntp_with(d.path(), true, &fail).unwrap();
        assert_eq!(
            active_ntp(d.path()).as_deref(),
            Some("chronyd"),
            "the boot link is the source of truth"
        );
    }

    #[test]
    fn configured_ntp_services_come_from_the_config_file() {
        let d = root();
        assert_eq!(ntp_candidates(d.path()), vec!["chronyd"]);
        fs::write(
            d.path().join("etc/dinit-settingsd.conf"),
            "ntp-services=ntpd, chronyd\n",
        )
        .unwrap();
        assert_eq!(ntp_candidates(d.path()), vec!["ntpd", "chronyd"]);
    }
}
