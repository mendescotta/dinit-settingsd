use crate::auth::{check, io_err, Ctx};
use crate::files;
use std::sync::Arc;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::{fdo, interface, Connection};

pub struct Hostname(pub Arc<Ctx>);

impl Hostname {
    fn static_name(&self) -> String {
        files::read(&files::path(&self.0.root, "/etc/hostname"))
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .unwrap_or("")
            .to_string()
    }
    fn info(&self, key: &str) -> String {
        files::env_get(&files::path(&self.0.root, "/etc/machine-info"), key).unwrap_or_default()
    }
    fn set_info(&self, key: &str, v: &str) -> fdo::Result<()> {
        let v = if v.is_empty() { None } else { Some(v) };
        files::env_set(&files::path(&self.0.root, "/etc/machine-info"), key, v)
            .map_err(io_err)
            .map(|_| ())
    }
    fn dmi(&self, f: &str) -> String {
        files::read(&files::path(
            &self.0.root,
            &format!("/sys/class/dmi/id/{f}"),
        ))
        .trim()
        .to_string()
    }
    fn os_release(&self, key: &str) -> String {
        let r = files::path(&self.0.root, "/etc/os-release");
        let r = if r.exists() {
            r
        } else {
            files::path(&self.0.root, "/usr/lib/os-release")
        };
        files::env_get(&r, key).unwrap_or_default()
    }
    fn uname(&self, f: fn(&libc::utsname) -> &[libc::c_char]) -> String {
        let mut u: libc::utsname = unsafe { std::mem::zeroed() };
        unsafe { libc::uname(&mut u) };
        let s = f(&u);
        let bytes: Vec<u8> = s
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as u8)
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
    fn live_hostname(&self) -> String {
        self.uname(|u| &u.nodename)
    }
}

pub fn valid_hostname(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 64
        && !h.starts_with(['.', '-'])
        && !h.ends_with(['.', '-'])
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

#[interface(name = "org.freedesktop.hostname1")]
impl Hostname {
    #[zbus(property)]
    fn hostname(&self) -> String {
        self.0.touch();
        let live = self.live_hostname();
        if self.0.test_mode || live.is_empty() {
            self.static_name()
        } else {
            live
        }
    }
    #[zbus(property)]
    fn static_hostname(&self) -> String {
        self.0.touch();
        self.static_name()
    }
    #[zbus(property)]
    fn pretty_hostname(&self) -> String {
        self.info("PRETTY_HOSTNAME")
    }
    #[zbus(property)]
    fn icon_name(&self) -> String {
        let n = self.info("ICON_NAME");
        if n.is_empty() {
            match self.chassis().as_str() {
                "" => "computer".into(),
                c => format!("computer-{c}"),
            }
        } else {
            n
        }
    }
    #[zbus(property)]
    fn chassis(&self) -> String {
        let c = self.info("CHASSIS");
        if !c.is_empty() {
            return c;
        }
        match self.dmi("chassis_type").as_str() {
            "8" | "9" | "10" | "14" => "laptop".into(),
            "3" | "4" | "5" | "6" | "7" | "13" | "15" | "16" | "23" | "24" | "35" => {
                "desktop".into()
            }
            "11" => "handset".into(),
            "30" => "tablet".into(),
            "17" | "25" | "28" | "29" => "server".into(),
            "31" | "32" => "convertible".into(),
            _ => String::new(),
        }
    }
    #[zbus(property)]
    fn deployment(&self) -> String {
        self.info("DEPLOYMENT")
    }
    #[zbus(property)]
    fn location(&self) -> String {
        self.info("LOCATION")
    }
    #[zbus(property)]
    fn kernel_name(&self) -> String {
        self.uname(|u| &u.sysname)
    }
    #[zbus(property)]
    fn kernel_release(&self) -> String {
        self.uname(|u| &u.release)
    }
    #[zbus(property)]
    fn operating_system_pretty_name(&self) -> String {
        self.os_release("PRETTY_NAME")
    }
    #[zbus(property)]
    fn hardware_vendor(&self) -> String {
        self.dmi("sys_vendor")
    }
    #[zbus(property)]
    fn hardware_model(&self) -> String {
        self.dmi("product_name")
    }
    #[zbus(property)]
    fn home_url(&self) -> String {
        self.os_release("HOME_URL")
    }

    async fn set_hostname(
        &self,
        hostname: String,
        interactive: bool,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] em: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        check(
            &self.0,
            conn,
            &hdr,
            "org.freedesktop.hostname1.set-hostname",
            interactive,
        )
        .await?;
        if !hostname.is_empty() && !valid_hostname(&hostname) {
            return Err(fdo::Error::InvalidArgs(format!(
                "Invalid hostname '{hostname}'"
            )));
        }
        if !self.0.test_mode {
            let name = if hostname.is_empty() {
                self.static_name()
            } else {
                hostname
            };
            let rc = unsafe { libc::sethostname(name.as_ptr().cast(), name.len()) };
            if rc != 0 {
                return Err(io_err(std::io::Error::last_os_error()));
            }
        }
        self.hostname_changed(&em).await?;
        Ok(())
    }

    async fn set_static_hostname(
        &self,
        hostname: String,
        interactive: bool,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] em: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        check(
            &self.0,
            conn,
            &hdr,
            "org.freedesktop.hostname1.set-static-hostname",
            interactive,
        )
        .await?;
        if !hostname.is_empty() && !valid_hostname(&hostname) {
            return Err(fdo::Error::InvalidArgs(format!(
                "Invalid hostname '{hostname}'"
            )));
        }
        let p = files::path(&self.0.root, "/etc/hostname");
        if hostname.is_empty() {
            let _ = std::fs::remove_file(&p);
        } else {
            files::atomic_write(&p, &format!("{hostname}\n")).map_err(io_err)?;
        }
        self.static_hostname_changed(&em).await?;
        Ok(())
    }

    async fn set_pretty_hostname(
        &self,
        hostname: String,
        interactive: bool,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] em: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        check(
            &self.0,
            conn,
            &hdr,
            "org.freedesktop.hostname1.set-machine-info",
            interactive,
        )
        .await?;
        self.set_info("PRETTY_HOSTNAME", &hostname)?;
        self.pretty_hostname_changed(&em).await?;
        Ok(())
    }

    async fn set_icon_name(
        &self,
        icon: String,
        interactive: bool,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] em: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        check(
            &self.0,
            conn,
            &hdr,
            "org.freedesktop.hostname1.set-machine-info",
            interactive,
        )
        .await?;
        self.set_info("ICON_NAME", &icon)?;
        self.icon_name_changed(&em).await?;
        Ok(())
    }

    async fn set_chassis(
        &self,
        chassis: String,
        interactive: bool,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] em: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        check(
            &self.0,
            conn,
            &hdr,
            "org.freedesktop.hostname1.set-machine-info",
            interactive,
        )
        .await?;
        const OK: &[&str] = &[
            "",
            "desktop",
            "laptop",
            "convertible",
            "server",
            "tablet",
            "handset",
            "watch",
            "embedded",
            "vm",
            "container",
        ];
        if !OK.contains(&chassis.as_str()) {
            return Err(fdo::Error::InvalidArgs(format!(
                "Invalid chassis '{chassis}'"
            )));
        }
        self.set_info("CHASSIS", &chassis)?;
        self.chassis_changed(&em).await?;
        Ok(())
    }

    async fn set_deployment(
        &self,
        deployment: String,
        interactive: bool,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] em: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        check(
            &self.0,
            conn,
            &hdr,
            "org.freedesktop.hostname1.set-machine-info",
            interactive,
        )
        .await?;
        self.set_info("DEPLOYMENT", &deployment)?;
        self.deployment_changed(&em).await?;
        Ok(())
    }

    async fn set_location(
        &self,
        location: String,
        interactive: bool,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] em: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        check(
            &self.0,
            conn,
            &hdr,
            "org.freedesktop.hostname1.set-machine-info",
            interactive,
        )
        .await?;
        self.set_info("LOCATION", &location)?;
        self.location_changed(&em).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostnames() {
        assert!(valid_hostname("void"));
        assert!(valid_hostname("a-b.c1"));
        assert!(!valid_hostname(""));
        assert!(!valid_hostname("-x"));
        assert!(!valid_hostname("x y"));
        assert!(!valid_hostname("a/b"));
    }
}
