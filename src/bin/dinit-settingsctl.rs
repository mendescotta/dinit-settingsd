use std::process::exit;
use zbus::blocking::{Connection, Proxy};

extern "C" {
    fn tzset();
}

type R<T> = Result<T, Box<dyn std::error::Error>>;

const HOSTNAME: (&str, &str) = ("org.freedesktop.hostname1", "/org/freedesktop/hostname1");
const TIMEDATE: (&str, &str) = ("org.freedesktop.timedate1", "/org/freedesktop/timedate1");
const LOCALE: (&str, &str) = ("org.freedesktop.locale1", "/org/freedesktop/locale1");

fn proxy<'a>(c: &'a Connection, t: (&'static str, &'static str)) -> R<Proxy<'a>> {
    Ok(Proxy::new(c, t.0, t.1, t.0)?)
}

fn usage(tool: &str) -> ! {
    let text = match tool {
        "timedatectl" => "timedatectl [status|show|set-timezone TZ|list-timezones|set-ntp BOOL|set-local-rtc BOOL]",
        "hostnamectl" => "hostnamectl [status|hostname [NAME]|set-hostname [--static|--pretty|--transient] NAME|set-icon-name|set-chassis|set-deployment|set-location VALUE]",
        "localectl" => "localectl [status|list-locales|set-locale VAR=VALUE...|set-keymap MAP|set-x11-keymap LAYOUT [MODEL [VARIANT [OPTIONS]]]]",
        _ => "dinit-settingsctl timedatectl|hostnamectl|localectl [args]\n(or invoke through a timedatectl/hostnamectl/localectl symlink)",
    };
    eprintln!("{text}");
    exit(2);
}

fn parse_bool(s: &str) -> R<bool> {
    match s {
        "1" | "true" | "yes" | "on" | "y" | "t" => Ok(true),
        "0" | "false" | "no" | "off" | "n" | "f" => Ok(false),
        _ => Err(format!("Failed to parse boolean '{s}'").into()),
    }
}

fn arg<'a>(args: &'a [String], i: usize, tool: &str) -> &'a str {
    args.get(i)
        .map(String::as_str)
        .unwrap_or_else(|| usage(tool))
}

fn fmt_time(usec: u64, utc: bool) -> String {
    if usec == 0 {
        return "n/a".into();
    }
    let t = (usec / 1_000_000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let mut buf = [0u8; 96];
    unsafe {
        if utc {
            libc::gmtime_r(&t, &mut tm);
        } else {
            tzset();
            libc::localtime_r(&t, &mut tm);
        }
        let n = libc::strftime(
            buf.as_mut_ptr().cast(),
            buf.len(),
            c"%a %Y-%m-%d %H:%M:%S %Z".as_ptr(),
            &tm,
        );
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }
}

fn yes(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

fn timedatectl(c: &Connection, args: &[String]) -> R<()> {
    let p = proxy(c, TIMEDATE)?;
    match args.first().map(String::as_str).unwrap_or("status") {
        "status" => {
            let tz: String = p.get_property("Timezone")?;
            let now: u64 = p.get_property("TimeUSec")?;
            let rtc: u64 = p.get_property("RTCTimeUSec")?;
            let sync: bool = p.get_property("NTPSynchronized")?;
            let ntp: bool = p.get_property("NTP")?;
            let can: bool = p.get_property("CanNTP")?;
            let local: bool = p.get_property("LocalRTC")?;
            println!("               Local time: {}", fmt_time(now, false));
            println!("           Universal time: {}", fmt_time(now, true));
            println!("                 RTC time: {}", fmt_time(rtc, true));
            println!("                Time zone: {tz}");
            println!("System clock synchronized: {}", yes(sync));
            println!(
                "              NTP service: {}",
                if !can {
                    "n/a"
                } else if ntp {
                    "active"
                } else {
                    "inactive"
                }
            );
            println!("          RTC in local TZ: {}", yes(local));
        }
        "show" => {
            println!("Timezone={}", p.get_property::<String>("Timezone")?);
            for k in ["LocalRTC", "CanNTP", "NTP", "NTPSynchronized"] {
                println!("{k}={}", yes(p.get_property::<bool>(k)?));
            }
            for k in ["TimeUSec", "RTCTimeUSec"] {
                println!("{k}={}", fmt_time(p.get_property::<u64>(k)?, false));
            }
        }
        "set-timezone" => p
            .call_method("SetTimezone", &(arg(args, 1, "timedatectl"), true))?
            .body()
            .deserialize::<()>()?,
        "list-timezones" => {
            let l: Vec<String> = p.call("ListTimezones", &())?;
            println!("{}", l.join("\n"));
        }
        "set-ntp" => p.call("SetNTP", &(parse_bool(arg(args, 1, "timedatectl"))?, true))?,
        "set-local-rtc" => p.call(
            "SetLocalRTC",
            &(parse_bool(arg(args, 1, "timedatectl"))?, false, true),
        )?,
        "set-time" => {
            return Err("set-time is not supported (dinit-settingsd has no SetTime)".into())
        }
        _ => usage("timedatectl"),
    }
    Ok(())
}

fn hostnamectl(c: &Connection, args: &[String]) -> R<()> {
    let p = proxy(c, HOSTNAME)?;
    let get = |k: &str| p.get_property::<String>(k).unwrap_or_default();
    match args.first().map(String::as_str).unwrap_or("status") {
        "status" => {
            let rows = [
                ("Static hostname", "StaticHostname"),
                ("Pretty hostname", "PrettyHostname"),
                ("Transient hostname", "Hostname"),
                ("Icon name", "IconName"),
                ("Chassis", "Chassis"),
                ("Deployment", "Deployment"),
                ("Location", "Location"),
                ("Operating System", "OperatingSystemPrettyName"),
                ("Kernel", ""),
                ("Hardware Vendor", "HardwareVendor"),
                ("Hardware Model", "HardwareModel"),
            ];
            for (label, key) in rows {
                let v = if label == "Kernel" {
                    format!("{} {}", get("KernelName"), get("KernelRelease"))
                } else {
                    get(key)
                };
                if !v.trim().is_empty() {
                    println!("{label:>20}: {v}");
                }
            }
        }
        "hostname" if args.len() == 1 => println!("{}", get("Hostname")),
        "hostname" | "set-hostname" => {
            let mut which = (false, false, false);
            let mut name = None;
            for a in &args[1..] {
                match a.as_str() {
                    "--static" => which.0 = true,
                    "--pretty" => which.1 = true,
                    "--transient" => which.2 = true,
                    n => name = Some(n),
                }
            }
            let name = name.unwrap_or_else(|| usage("hostnamectl"));
            if which == (false, false, false) {
                which = (true, false, true);
            }
            if which.1 {
                p.call::<_, _, ()>("SetPrettyHostname", &(name, true))?;
            }
            if which.0 {
                p.call::<_, _, ()>("SetStaticHostname", &(name, true))?;
            }
            if which.2 {
                p.call::<_, _, ()>("SetHostname", &(name, true))?;
            }
        }
        "set-icon-name" => p.call("SetIconName", &(arg(args, 1, "hostnamectl"), true))?,
        "set-chassis" => p.call("SetChassis", &(arg(args, 1, "hostnamectl"), true))?,
        "set-deployment" => p.call("SetDeployment", &(arg(args, 1, "hostnamectl"), true))?,
        "set-location" => p.call("SetLocation", &(arg(args, 1, "hostnamectl"), true))?,
        _ => usage("hostnamectl"),
    }
    Ok(())
}

fn localectl(c: &Connection, args: &[String]) -> R<()> {
    let p = proxy(c, LOCALE)?;
    match args.first().map(String::as_str).unwrap_or("status") {
        "status" => {
            let l: Vec<String> = p.get_property("Locale")?;
            let s = |k: &str| p.get_property::<String>(k).unwrap_or_default();
            let or_unset = |v: String| {
                if v.is_empty() {
                    "(unset)".to_string()
                } else {
                    v
                }
            };
            println!(
                "   System Locale: {}",
                l.first().map(String::as_str).unwrap_or("n/a")
            );
            for x in l.iter().skip(1) {
                println!("                  {x}");
            }
            println!("       VC Keymap: {}", or_unset(s("VConsoleKeymap")));
            println!("      X11 Layout: {}", or_unset(s("X11Layout")));
            println!("       X11 Model: {}", or_unset(s("X11Model")));
            println!("     X11 Variant: {}", or_unset(s("X11Variant")));
            println!("     X11 Options: {}", or_unset(s("X11Options")));
        }
        "list-locales" => {
            let o = std::process::Command::new("locale").arg("-a").output()?;
            let out = String::from_utf8_lossy(&o.stdout);
            for l in out.lines().filter(|l| *l != "C" && *l != "POSIX") {
                println!("{l}");
            }
        }
        "set-locale" => {
            if args.len() < 2 {
                usage("localectl");
            }
            let items: Vec<String> = args[1..]
                .iter()
                .map(|a| {
                    if a.contains('=') {
                        a.clone()
                    } else {
                        format!("LANG={a}")
                    }
                })
                .collect();
            p.call("SetLocale", &(items, true))?
        }
        "set-keymap" => {
            let map = arg(args, 1, "localectl");
            let toggle = args.get(2).map(String::as_str).unwrap_or("");
            p.call("SetVConsoleKeyboard", &(map, toggle, false, true))?
        }
        "set-x11-keymap" => {
            let g = |i| args.get(i).map(String::as_str).unwrap_or("");
            p.call(
                "SetX11Keyboard",
                &(arg(args, 1, "localectl"), g(2), g(3), g(4), false, true),
            )?
        }
        _ => usage("localectl"),
    }
    Ok(())
}

fn main() {
    let mut argv: Vec<String> = std::env::args().collect();
    let invoked = std::path::Path::new(&argv[0])
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let tool = match invoked.as_str() {
        "timedatectl" | "hostnamectl" | "localectl" => invoked,
        _ => {
            if argv.len() < 2 {
                usage("");
            }
            argv.remove(1)
        }
    };
    let args = &argv[1..];
    if args.first().is_some_and(|a| a == "-h" || a == "--help") {
        usage(&tool);
    }
    let c = match Connection::system() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to system bus: {e}");
            exit(1);
        }
    };
    let r = match tool.as_str() {
        "timedatectl" => timedatectl(&c, args),
        "hostnamectl" => hostnamectl(&c, args),
        "localectl" => localectl(&c, args),
        _ => usage(""),
    };
    if let Err(e) = r {
        eprintln!("{e}");
        exit(1);
    }
}
