//! Console keymap <-> X11 layout conversion for the `convert` flags of
//! SetVConsoleKeyboard / SetX11Keyboard. Covers common layouts only.

pub struct X11 {
    pub layout: &'static str,
    pub model: &'static str,
    pub variant: &'static str,
    pub options: &'static str,
}

// (console keymap, X11 layout, X11 variant, X11 options)
const MAP: &[(&str, &str, &str, &str)] = &[
    ("us", "us", "", ""),
    ("dvorak", "us", "dvorak", ""),
    ("colemak", "us", "colemak", ""),
    ("uk", "gb", "", ""),
    ("ie", "ie", "", ""),
    ("de", "de", "", ""),
    ("de-latin1", "de", "", ""),
    ("de-latin1-nodeadkeys", "de", "nodeadkeys", ""),
    ("fr", "fr", "", ""),
    ("fr-latin1", "fr", "", ""),
    ("fr-latin9", "fr", "latin9", ""),
    ("be-latin1", "be", "", ""),
    ("es", "es", "", ""),
    ("pt-latin1", "pt", "", ""),
    ("br-abnt2", "br", "abnt2", ""),
    ("it", "it", "", ""),
    ("it2", "it", "", ""),
    ("nl", "nl", "", ""),
    ("no", "no", "", ""),
    ("no-latin1", "no", "", ""),
    ("dk", "dk", "", ""),
    ("dk-latin1", "dk", "", ""),
    ("sv-latin1", "se", "", ""),
    ("fi", "fi", "", ""),
    ("is-latin1", "is", "", ""),
    ("pl2", "pl", "", ""),
    ("cz", "cz", "", ""),
    ("cz-lat2", "cz", "", ""),
    ("sk-qwerty", "sk", "qwerty", ""),
    ("hu", "hu", "", ""),
    ("slovene", "si", "", ""),
    ("croat", "hr", "", ""),
    ("ro", "ro", "", ""),
    ("bg_bds-utf8", "bg", "", "grp:shift_toggle"),
    ("ru", "ru", "", "grp:shift_toggle"),
    ("ua", "ua", "", "grp:shift_toggle"),
    ("gr", "gr", "", "grp:shift_toggle"),
    ("tr_q-latin5", "tr", "", ""),
    ("il", "il", "", ""),
    ("lt", "lt", "", ""),
    ("lv", "lv", "", ""),
    ("et", "ee", "", ""),
    ("cf", "ca", "fr", ""),
    ("ca", "ca", "", ""),
    ("jp106", "jp", "", ""),
    ("sg", "ch", "", ""),
    ("fr_CH", "ch", "fr", ""),
    ("la-latin1", "latam", "", ""),
];

fn first(list: &str) -> &str {
    list.split(',').next().unwrap_or("")
}

pub fn console_to_x11(keymap: &str) -> Option<X11> {
    MAP.iter().find(|m| m.0 == keymap).map(|m| X11 {
        layout: m.1,
        model: if m.1 == "jp" { "jp106" } else { "pc105" },
        variant: m.2,
        options: m.3,
    })
}

pub fn x11_to_console(layout: &str, variant: &str) -> Option<&'static str> {
    let (layout, variant) = (first(layout), first(variant));
    MAP.iter()
        .find(|m| m.1 == layout && m.2 == variant)
        .or_else(|| MAP.iter().find(|m| m.1 == layout && m.2.is_empty()))
        .map(|m| m.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_maps_to_x11() {
        let x = console_to_x11("de-latin1-nodeadkeys").unwrap();
        assert_eq!(
            (x.layout, x.variant, x.model),
            ("de", "nodeadkeys", "pc105")
        );
        assert_eq!(console_to_x11("uk").unwrap().layout, "gb");
        assert!(console_to_x11("nope").is_none());
    }

    #[test]
    fn x11_maps_to_console() {
        assert_eq!(x11_to_console("us", "dvorak"), Some("dvorak"));
        assert_eq!(x11_to_console("gb", ""), Some("uk"));
        assert_eq!(x11_to_console("us,ru", ",phonetic"), Some("us"));
        assert_eq!(x11_to_console("de", "unknown-variant"), Some("de"));
        assert_eq!(x11_to_console("zz", ""), None);
    }
}
