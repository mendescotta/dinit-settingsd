use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub fn path(root: &Path, p: &str) -> PathBuf {
    root.join(p.trim_start_matches('/'))
}

pub fn atomic_write(path: &Path, data: &str) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let mode = fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o7777)
        .unwrap_or(0o644);
    let tmp = dir.join(format!(
        ".{}.settingsd.{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("tmp"),
        std::process::id()
    ));
    let mut f = fs::File::create(&tmp)?;
    f.write_all(data.as_bytes())?;
    f.sync_all()?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
    fs::rename(&tmp, path)
}

pub fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    let b = v.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        v[1..v.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else {
        v.to_string()
    }
}

fn quote(v: &str) -> String {
    if v.chars()
        .all(|c| c.is_ascii_alphanumeric() || "._-/:@+,".contains(c))
    {
        v.to_string()
    } else {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

fn assignment(line: &str) -> Option<(&str, &str, &str)> {
    let l = line.trim();
    if l.starts_with('#') {
        return None;
    }
    let (prefix, rest) = match l.strip_prefix("export ") {
        Some(r) => ("export ", r.trim_start()),
        None => ("", l),
    };
    let (k, v) = rest.split_once('=')?;
    Some((prefix, k.trim(), v))
}

fn quotes_balanced(v: &str) -> bool {
    let v = v.trim();
    match v.chars().next() {
        Some(q @ ('"' | '\'')) => {
            v.len() >= 2 && v.ends_with(q) && !v[..v.len() - 1].ends_with('\\')
        }
        _ => true,
    }
}

pub fn env_get(path: &Path, key: &str) -> Option<String> {
    let mut found = None;
    for line in read(path).lines() {
        if let Some((_, k, v)) = assignment(line) {
            if k == key {
                found = Some(unquote(v));
            }
        }
    }
    found
}

/// Set (Some) or remove (None) KEY in a shell-style KEY=VALUE file.
/// Only uncommented assignments are replaced; a missing key is appended.
/// Returns Ok(true) if the file content changed.
pub fn env_set(path: &Path, key: &str, value: Option<&str>) -> io::Result<bool> {
    let old = read(path);
    let mut out = Vec::new();
    let mut done = false;
    for line in old.lines() {
        let hit = assignment(line).filter(|(_, k, _)| *k == key);
        if let Some((prefix, _, raw)) = hit {
            if !quotes_balanced(raw) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "refusing to modify {}: {key} is not a simple assignment",
                        path.display()
                    ),
                ));
            }
            if let (Some(v), false) = (value, done) {
                out.push(format!("{prefix}{key}={}", quote(v)));
                done = true;
            }
        } else {
            out.push(line.to_string());
        }
    }
    if let (Some(v), false) = (value, done) {
        out.push(format!("{}={}", key, quote(v)));
    }
    let mut new = out.join("\n");
    if !new.is_empty() {
        new.push('\n');
    }
    if new == old {
        return Ok(false);
    }
    if new.is_empty() && !path.exists() {
        return Ok(false);
    }
    atomic_write(path, &new)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("rc.conf");
        fs::write(&p, "#KEYMAP=\"us\"\nKEYMAP=no\nFOO=bar\n").unwrap();
        assert_eq!(env_get(&p, "KEYMAP").as_deref(), Some("no"));
        assert!(env_set(&p, "KEYMAP", Some("de-latin1")).unwrap());
        assert_eq!(read(&p), "#KEYMAP=\"us\"\nKEYMAP=de-latin1\nFOO=bar\n");
        assert!(!env_set(&p, "KEYMAP", Some("de-latin1")).unwrap());
        env_set(&p, "PRETTY", Some("My Laptop \"x\"")).unwrap();
        assert_eq!(env_get(&p, "PRETTY").as_deref(), Some("My Laptop \"x\""));
        env_set(&p, "KEYMAP", None).unwrap();
        assert_eq!(env_get(&p, "KEYMAP"), None);
    }

    #[test]
    fn env_handles_export_and_refuses_multiline() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("rc.conf");
        fs::write(&p, "export KEYMAP=us\n").unwrap();
        assert_eq!(env_get(&p, "KEYMAP").as_deref(), Some("us"));
        env_set(&p, "KEYMAP", Some("de")).unwrap();
        assert_eq!(read(&p), "export KEYMAP=de\n");
        fs::write(&p, "KEYMAP=\"a\nb\"\n").unwrap();
        assert!(env_set(&p, "KEYMAP", Some("de")).is_err());
        assert_eq!(read(&p), "KEYMAP=\"a\nb\"\n");
    }

    #[test]
    fn env_set_creates_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/machine-info");
        env_set(&p, "CHASSIS", Some("laptop")).unwrap();
        assert_eq!(read(&p), "CHASSIS=laptop\n");
    }
}
