mod auth;
mod files;
mod hostname;
mod keymap;
mod locale;
mod timedate;

use auth::Ctx;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

const IDLE_SECS: u64 = 60;

fn main() -> zbus::Result<()> {
    let mut root = PathBuf::from("/");
    let mut session = false;
    let mut no_auth = false;
    let mut persist = false;
    let mut read_only = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--root" => root = PathBuf::from(args.next().expect("--root needs a path")),
            "--bus" => session = args.next().as_deref() == Some("session"),
            "--no-auth" => no_auth = true,
            "--persist" => persist = true,
            "--read-only" => read_only = true,
            "--ntp-service" => {
                let svc = args.next().expect("--ntp-service needs a service name");
                timedate::set_ntp_override(svc);
            }
            "--version" => {
                println!("dinit-settingsd {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-h" | "--help" => {
                println!(
                    "dinit-settingsd [--read-only] [--ntp-service NAME] [--version]\n\
                     \t[--root DIR] [--bus system|session] [--no-auth] [--persist]"
                );
                return Ok(());
            }
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let test_mode = root.as_path() != std::path::Path::new("/");
    if (test_mode || no_auth) && !session {
        eprintln!("--root/--no-auth are only allowed with --bus session");
        std::process::exit(2);
    }
    let ctx = Arc::new(Ctx {
        root,
        no_auth,
        read_only,
        test_mode,
        last_activity: AtomicU64::new(0),
    });
    ctx.touch();

    async_io::block_on(async {
        let b = if session {
            zbus::connection::Builder::session()?
        } else {
            zbus::connection::Builder::system()?
        };
        let _conn = b
            .name("org.freedesktop.hostname1")?
            .name("org.freedesktop.timedate1")?
            .name("org.freedesktop.locale1")?
            .serve_at(
                "/org/freedesktop/hostname1",
                hostname::Hostname(ctx.clone()),
            )?
            .serve_at(
                "/org/freedesktop/timedate1",
                timedate::Timedate(ctx.clone()),
            )?
            .serve_at("/org/freedesktop/locale1", locale::Locale(ctx.clone()))?
            .build()
            .await?;
        loop {
            async_io::Timer::after(Duration::from_secs(5)).await;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if !persist && now.saturating_sub(ctx.last_activity.load(Ordering::Relaxed)) > IDLE_SECS
            {
                return Ok(());
            }
        }
    })
}
