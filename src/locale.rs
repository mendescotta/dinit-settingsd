use crate::auth::{check, Ctx};
use crate::{files, keymap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::{fdo, interface, Connection};

pub struct Locale(pub Arc<Ctx>);

const LOCALE_KEYS: &[&str] = &[
    "LANG",
    "LANGUAGE",
    "LC_CTYPE",
    "LC_NUMERIC",
    "LC_TIME",
    "LC_COLLATE",
    "LC_MONETARY",
    "LC_MESSAGES",
    "LC_PAPER",
    "LC_NAME",
    "LC_ADDRESS",
    "LC_TELEPHONE",
    "LC_MEASUREMENT",
    "LC_IDENTIFICATION",
    "LC_ALL",
];

pub fn get_locale(root: &Path) -> Vec<String> {
    let p = files::path(root, "/etc/locale.conf");
    LOCALE_KEYS
        .iter()
        .filter_map(|k| {
            files::env_get(&p, k)
                .filter(|v| !v.is_empty())
                .map(|v| format!("{k}={v}"))
        })
        .collect()
}

pub fn parse_assignments(items: &[String]) -> Result<Vec<(String, String)>, String> {
    items
        .iter()
        .map(|i| {
            let (k, v) = i
                .split_once('=')
                .ok_or_else(|| format!("Invalid locale assignment '{i}'"))?;
            if !LOCALE_KEYS.contains(&k) {
                return Err(format!("Invalid locale variable '{k}'"));
            }
            if v.chars().any(|c| c.is_control()) {
                return Err(format!("Invalid value for {k}"));
            }
            Ok((k.to_string(), v.to_string()))
        })
        .collect()
}

fn norm(l: &str) -> String {
    l.to_lowercase().replace("utf-8", "utf8")
}

fn generated(l: &str) -> bool {
    if l.is_empty() || l == "C" || l == "POSIX" || l.starts_with("C.") {
        return true;
    }
    Command::new("locale")
        .arg("-a")
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|x| norm(x) == norm(l))
        })
        .unwrap_or(true)
}

pub fn set_locale(root: &Path, items: &[String], verify: bool) -> Result<(), String> {
    let kv = parse_assignments(items)?;
    if verify {
        for (k, v) in &kv {
            if k != "LANGUAGE" && !generated(v) {
                return Err(format!(
                    "Locale '{v}' is not generated; enable it in /etc/default/libc-locales and run xbps-reconfigure -f glibc-locales"
                ));
            }
        }
    }
    let p = files::path(root, "/etc/locale.conf");
    for k in LOCALE_KEYS {
        let v = kv
            .iter()
            .find(|(kk, _)| kk == k)
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty());
        files::env_set(&p, k, v).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn keymap(root: &Path) -> String {
    files::env_get(&files::path(root, "/etc/rc.conf"), "KEYMAP")
        .or_else(|| files::env_get(&files::path(root, "/etc/vconsole.conf"), "KEYMAP"))
        .unwrap_or_default()
}

pub fn set_keymap(root: &Path, keymap: &str) -> Result<(), String> {
    if keymap
        .chars()
        .any(|c| !(c.is_ascii_alphanumeric() || "-_.".contains(c)))
    {
        return Err(format!("Invalid keymap '{keymap}'"));
    }
    let v = if keymap.is_empty() {
        None
    } else {
        Some(keymap)
    };
    files::env_set(&files::path(root, "/etc/rc.conf"), "KEYMAP", v).map_err(|e| e.to_string())?;
    Ok(())
}

fn x11_file(root: &Path) -> PathBuf {
    for n in ["00-keyboard.conf", "30-keyboard.conf"] {
        let p = files::path(root, &format!("/etc/X11/xorg.conf.d/{n}"));
        if p.exists() {
            return p;
        }
    }
    files::path(root, "/etc/X11/xorg.conf.d/00-keyboard.conf")
}

fn x11_opt(content: &str, name: &str) -> String {
    for l in content.lines() {
        let t: Vec<&str> = l.split('"').collect();
        if t.len() >= 4 && t[0].trim() == "Option" && t[1] == name {
            return t[3].to_string();
        }
    }
    String::new()
}

pub fn get_x11(root: &Path) -> [String; 4] {
    let c = files::read(&x11_file(root));
    ["XkbLayout", "XkbModel", "XkbVariant", "XkbOptions"].map(|n| x11_opt(&c, n))
}

pub fn set_x11(
    root: &Path,
    layout: &str,
    model: &str,
    variant: &str,
    options: &str,
) -> Result<(), String> {
    for v in [layout, model, variant, options] {
        if v.chars().any(|c| c == '"' || c.is_control()) {
            return Err("Invalid keyboard setting".into());
        }
    }
    let p = x11_file(root);
    if [layout, model, variant, options]
        .iter()
        .all(|s| s.is_empty())
    {
        let _ = std::fs::remove_file(&p);
        return Ok(());
    }
    let mut s = String::from(
        "# Written by dinit-settingsd\n\nSection \"InputClass\"\n\tIdentifier \"keyboard\"\n\tMatchIsKeyboard \"on\"\n",
    );
    for (n, v) in [
        ("XkbLayout", layout),
        ("XkbModel", model),
        ("XkbVariant", variant),
        ("XkbOptions", options),
    ] {
        if !v.is_empty() {
            s.push_str(&format!("\tOption \"{n}\" \"{v}\"\n"));
        }
    }
    s.push_str("EndSection\n");
    files::atomic_write(&p, &s).map_err(|e| e.to_string())
}

impl Locale {
    async fn emit_x11_changed(&self, em: &SignalEmitter<'_>) -> zbus::Result<()> {
        self.x11_layout_changed(em).await?;
        self.x11_model_changed(em).await?;
        self.x11_variant_changed(em).await?;
        self.x11_options_changed(em).await
    }
}

#[allow(clippy::too_many_arguments)]
#[interface(name = "org.freedesktop.locale1")]
impl Locale {
    #[zbus(property)]
    fn locale(&self) -> Vec<String> {
        self.0.touch();
        get_locale(&self.0.root)
    }
    #[zbus(property, name = "VConsoleKeymap")]
    fn vconsole_keymap(&self) -> String {
        keymap(&self.0.root)
    }
    #[zbus(property, name = "VConsoleKeymapToggle")]
    fn vconsole_keymap_toggle(&self) -> String {
        String::new()
    }
    #[zbus(property, name = "X11Layout")]
    fn x11_layout(&self) -> String {
        get_x11(&self.0.root)[0].clone()
    }
    #[zbus(property, name = "X11Model")]
    fn x11_model(&self) -> String {
        get_x11(&self.0.root)[1].clone()
    }
    #[zbus(property, name = "X11Variant")]
    fn x11_variant(&self) -> String {
        get_x11(&self.0.root)[2].clone()
    }
    #[zbus(property, name = "X11Options")]
    fn x11_options(&self) -> String {
        get_x11(&self.0.root)[3].clone()
    }

    async fn set_locale(
        &self,
        locale: Vec<String>,
        interactive: bool,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] em: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        check(
            &self.0,
            conn,
            &hdr,
            "org.freedesktop.locale1.set-locale",
            interactive,
        )
        .await?;
        set_locale(&self.0.root, &locale, !self.0.test_mode).map_err(fdo::Error::InvalidArgs)?;
        self.locale_changed(&em).await?;
        Ok(())
    }

    #[zbus(name = "SetVConsoleKeyboard")]
    async fn set_vconsole_keyboard(
        &self,
        keymap: String,
        _keymap_toggle: String,
        convert: bool,
        interactive: bool,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] em: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        check(
            &self.0,
            conn,
            &hdr,
            "org.freedesktop.locale1.set-keyboard",
            interactive,
        )
        .await?;
        set_keymap(&self.0.root, &keymap).map_err(fdo::Error::InvalidArgs)?;
        self.v_console_keymap_changed(&em).await?;
        if convert {
            if let Some(x) = keymap::console_to_x11(&keymap) {
                set_x11(&self.0.root, x.layout, x.model, x.variant, x.options)
                    .map_err(fdo::Error::InvalidArgs)?;
                self.emit_x11_changed(&em).await?;
            }
        }
        Ok(())
    }

    #[zbus(name = "SetX11Keyboard")]
    async fn set_x11_keyboard(
        &self,
        layout: String,
        model: String,
        variant: String,
        options: String,
        convert: bool,
        interactive: bool,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] em: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        check(
            &self.0,
            conn,
            &hdr,
            "org.freedesktop.locale1.set-keyboard",
            interactive,
        )
        .await?;
        set_x11(&self.0.root, &layout, &model, &variant, &options)
            .map_err(fdo::Error::InvalidArgs)?;
        self.emit_x11_changed(&em).await?;
        if convert {
            if let Some(map) = keymap::x11_to_console(&layout, &variant) {
                set_keymap(&self.0.root, map).map_err(fdo::Error::InvalidArgs)?;
                self.v_console_keymap_changed(&em).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn locale_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        set_locale(
            d.path(),
            &["LANG=en_GB.UTF-8".into(), "LC_TIME=et_EE.UTF-8".into()],
            false,
        )
        .unwrap();
        assert_eq!(
            get_locale(d.path()),
            vec!["LANG=en_GB.UTF-8", "LC_TIME=et_EE.UTF-8"]
        );
        set_locale(d.path(), &["LANG=C.UTF-8".into()], false).unwrap();
        assert_eq!(get_locale(d.path()), vec!["LANG=C.UTF-8"]);
        assert!(set_locale(d.path(), &["PATH=/x".into()], false).is_err());
        assert!(set_locale(d.path(), &["nonsense".into()], false).is_err());
    }

    #[test]
    fn keymap_rc_conf() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("etc")).unwrap();
        fs::write(d.path().join("etc/rc.conf"), "#TIMEZONE=x\nKEYMAP=no\n").unwrap();
        assert_eq!(keymap(d.path()), "no");
        set_keymap(d.path(), "de-latin1").unwrap();
        assert_eq!(keymap(d.path()), "de-latin1");
        assert!(set_keymap(d.path(), "a b").is_err());
    }

    #[test]
    fn x11_reuses_existing_file() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("etc/X11/xorg.conf.d");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("30-keyboard.conf"),
            "Section \"InputClass\"\n        Option \"XkbLayout\" \"no\"\n        Option \"XkbVariant\" \"nodeadkeys\"\nEndSection\n",
        )
        .unwrap();
        assert_eq!(get_x11(d.path()), ["no", "", "nodeadkeys", ""]);
        set_x11(d.path(), "us", "", "", "ctrl:nocaps").unwrap();
        assert_eq!(get_x11(d.path()), ["us", "", "", "ctrl:nocaps"]);
        assert!(!dir.join("00-keyboard.conf").exists());
        set_x11(d.path(), "", "", "", "").unwrap();
        assert!(!dir.join("30-keyboard.conf").exists());
    }
}
