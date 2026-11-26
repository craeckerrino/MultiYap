//! Lesbare Benutzernamen: zu dunkle Namensfarben auf dunklem Hintergrund aufhellen (im hellen Modus zu helle
//! abdunkeln), bis der Kontrast mindestens 4,5:1 erreicht (WCAG AA für Text). Der Farbton bleibt erhalten, nur die
//! Helligkeit ändert sich.

const MIN_CONTRAST: f64 = 4.5;
/// Relative Leuchtdichte der Hintergründe (dunkel etwa #0a0a0a, hell #ffffff).
const DARK_BACKGROUND: f64 = 0.003;
const LIGHT_BACKGROUND: f64 = 1.0;

type Rgb = [f64; 3];

fn channel(value: f64) -> f64 {
    let c = value / 255.0;
    if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn luminance([r, g, b]: Rgb) -> f64 {
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

pub fn contrast_ratio(a: f64, b: f64) -> f64 {
    let (light, dark) = if a > b { (a, b) } else { (b, a) };
    (light + 0.05) / (dark + 0.05)
}

/// #rrggbb (Groß- oder Kleinbuchstaben).
pub fn parse_hex(color: &str) -> Option<[u8; 3]> {
    let hex = color.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some([(value >> 16) as u8, (value >> 8) as u8, value as u8])
}

fn to_hex([r, g, b]: Rgb) -> String {
    // Wie Math.round in JavaScript (bei .5 aufwärts).
    let byte = |v: f64| (v + 0.5).floor().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b))
}

fn rgb_to_hsl([r, g, b]: Rgb) -> [f64; 3] {
    let (rn, gn, bn) = (r / 255.0, g / 255.0, b / 255.0);
    let max = rn.max(gn).max(bn);
    let min = rn.min(gn).min(bn);
    let l = (max + min) / 2.0;
    if max == min {
        return [0.0, 0.0, l];
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == rn {
        (gn - bn) / d + if gn < bn { 6.0 } else { 0.0 }
    } else if max == gn {
        (bn - rn) / d + 2.0
    } else {
        (rn - gn) / d + 4.0
    };
    [h / 6.0, s, l]
}

fn hsl_to_rgb([h, s, l]: [f64; 3]) -> Rgb {
    if s == 0.0 {
        return [l * 255.0, l * 255.0, l * 255.0];
    }
    let hue = |p: f64, q: f64, t: f64| {
        let mut x = t;
        if x < 0.0 {
            x += 1.0;
        }
        if x > 1.0 {
            x -= 1.0;
        }
        if x < 1.0 / 6.0 {
            p + (q - p) * 6.0 * x
        } else if x < 1.0 / 2.0 {
            q
        } else if x < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - x) * 6.0
        } else {
            p
        }
    };
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    [hue(p, q, h + 1.0 / 3.0) * 255.0, hue(p, q, h) * 255.0, hue(p, q, h - 1.0 / 3.0) * 255.0]
}

fn background(dark: bool) -> f64 {
    if dark { DARK_BACKGROUND } else { LIGHT_BACKGROUND }
}

/// Die 15 Standardfarben von Twitch (Befehl „/color“ ohne Turbo oder Prime).
const DEFAULT_NAME_COLORS: [&str; 15] = [
    "#0000FF", "#FF7F50", "#1E90FF", "#00FF7F", "#9ACD32", "#008000", "#FF4500", "#FF0000", "#DAA520", "#FF69B4", "#5F9EA0", "#2E8B57",
    "#D2691E", "#8A2BE2", "#B22222",
];

/// Farbe für Nutzer ohne eigene Namensfarbe: Twitch liefert dann keine Farbe (leer), der Name blieb weiß. Jede Person
/// bekommt fest eine der Standardfarben, aus ihrer Nutzer-ID abgeleitet.
pub fn default_name_color(user_id: &str) -> &'static str {
    let key = user_id.parse::<u64>().unwrap_or_else(|_| user_id.bytes().fold(0u64, |sum, b| sum.wrapping_mul(31).wrapping_add(u64::from(b))));
    DEFAULT_NAME_COLORS[(key % DEFAULT_NAME_COLORS.len() as u64) as usize]
}

/// Ausreichend kontrastreiche Variante der Farbe (unveränderte Farbe, wenn sie reicht oder ungültig ist).
pub fn readable_name_color(color: &str, dark: bool) -> String {
    let Some([r, g, b]) = parse_hex(color) else { return color.to_string() };
    let rgb = [f64::from(r), f64::from(g), f64::from(b)];
    let background = background(dark);
    if contrast_ratio(luminance(rgb), background) >= MIN_CONTRAST {
        return color.to_string();
    }
    let [h, s, l] = rgb_to_hsl(rgb);
    // Helligkeit in kleinen Schritten anpassen, bis der Kontrast reicht.
    for step in 1..=100 {
        let step = f64::from(step) * 0.01;
        let lightness = if dark { (l + step).min(1.0) } else { (l - step).max(0.0) };
        let candidate = hsl_to_rgb([h, s, lightness]);
        if contrast_ratio(luminance(candidate), background) >= MIN_CONTRAST || lightness == 0.0 || lightness == 1.0 {
            return to_hex(candidate);
        }
    }
    color.to_string()
}

/// Kontrast einer Hex-Farbe zum Hintergrund (0 bei ungültiger Farbe).
pub fn contrast_to_background(color: &str, dark: bool) -> f64 {
    parse_hex(color).map_or(0.0, |[r, g, b]| contrast_ratio(luminance([f64::from(r), f64::from(g), f64::from(b)]), background(dark)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hellt_dunkle_farben_auf() {
        assert_eq!(default_name_color("15"), "#0000FF");
        assert_eq!(default_name_color("16"), default_name_color("16"));
        assert!(DEFAULT_NAME_COLORS.contains(&default_name_color("kein-zahlwert")));
        let fixed = readable_name_color("#1b1b5e", true);
        assert_ne!(fixed, "#1b1b5e");
        assert!(contrast_to_background(&fixed, true) >= 4.5);
    }

    #[test]
    fn laesst_lesbare_farben() {
        assert_eq!(readable_name_color("#2e8b57", true), "#2e8b57");
        assert_eq!(readable_name_color("#FFFFFF", true), "#FFFFFF");
        assert_eq!(readable_name_color("rot", true), "rot");
    }

    #[test]
    fn dunkelt_im_hellen_modus_ab() {
        let fixed = readable_name_color("#ffff66", false);
        assert!(contrast_to_background(&fixed, false) >= 4.5);
    }
}
