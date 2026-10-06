use std::collections::HashMap;
use zbus::message::Header;
use zbus::zvariant::Value;
use zbus::{fdo, Connection};

pub struct Ctx {
    pub root: std::path::PathBuf,
    pub no_auth: bool,
    pub read_only: bool,
    pub test_mode: bool,
    pub last_activity: std::sync::atomic::AtomicU64,
}

impl Ctx {
    pub fn touch(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.last_activity
            .store(now, std::sync::atomic::Ordering::Relaxed);
    }
}

pub fn io_err(e: std::io::Error) -> fdo::Error {
    fdo::Error::Failed(e.to_string())
}

pub async fn check(
    ctx: &Ctx,
    conn: &Connection,
    hdr: &Header<'_>,
    action: &str,
    interactive: bool,
) -> fdo::Result<()> {
    ctx.touch();
    if ctx.read_only {
        return Err(fdo::Error::NotSupported(
            "dinit-settingsd is running in read-only mode".into(),
        ));
    }
    if ctx.no_auth {
        return Ok(());
    }
    let sender = hdr
        .sender()
        .ok_or_else(|| fdo::Error::AccessDenied("no sender".into()))?
        .to_string();
    let mut subject_details: HashMap<&str, Value<'_>> = HashMap::new();
    subject_details.insert("name", Value::from(sender));
    let subject = ("system-bus-name", subject_details);
    let details: HashMap<&str, &str> = HashMap::new();
    let flags: u32 = if interactive { 1 } else { 0 };
    let reply = conn
        .call_method(
            Some("org.freedesktop.PolicyKit1"),
            "/org/freedesktop/PolicyKit1/Authority",
            Some("org.freedesktop.PolicyKit1.Authority"),
            "CheckAuthorization",
            &(subject, action, details, flags, ""),
        )
        .await
        .map_err(|e| fdo::Error::AuthFailed(format!("polkit: {e}")))?;
    let (authorized, _challenge, _d): (bool, bool, HashMap<String, String>) = reply
        .body()
        .deserialize()
        .map_err(|e| fdo::Error::AuthFailed(format!("polkit reply: {e}")))?;
    if authorized {
        Ok(())
    } else {
        Err(fdo::Error::AccessDenied(format!(
            "Not authorized to perform {action}"
        )))
    }
}
