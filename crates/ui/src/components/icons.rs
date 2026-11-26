//! Symbole aus lucide (wie lucide-react 1.73): gleiche Pfade, 24er-Raster, Strichstärke 2, Größe über Klassen.

use leptos::prelude::*;

fn icon(name: &'static str, class: &'static str, paths: &'static [&'static str]) -> impl IntoView {
    view! {
        <svg
            xmlns="http://www.w3.org/2000/svg"
            width="24"
            height="24"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            class=format!("lucide lucide-{name} {class}")
            aria-hidden="true"
        >
            {paths.iter().map(|d| view! { <path d=*d></path> }).collect_view()}
        </svg>
    }
}

pub fn chevron_down(class: &'static str) -> impl IntoView {
    icon("chevron-down", class, &["m6 9 6 6 6-6"])
}

pub fn chevron_up(class: &'static str) -> impl IntoView {
    icon("chevron-up", class, &["m18 15-6-6-6 6"])
}

pub fn check(class: &'static str) -> impl IntoView {
    icon("check", class, &["M20 6 9 17l-5-5"])
}

pub fn x(class: &'static str) -> impl IntoView {
    icon("x", class, &["M18 6 6 18", "m6 6 12 12"])
}

pub fn plus(class: &'static str) -> impl IntoView {
    icon("plus", class, &["M5 12h14", "M12 5v14"])
}

pub fn trash(class: &'static str) -> impl IntoView {
    icon("trash", class, &["M10 11v6", "M14 11v6", "M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6", "M3 6h18", "M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"])
}

pub fn arrow_up(class: &'static str) -> impl IntoView {
    icon("arrow-up", class, &["m5 12 7-7 7 7", "M12 19V5"])
}

pub fn arrow_down(class: &'static str) -> impl IntoView {
    icon("arrow-down", class, &["M12 5v14", "m19 12-7 7-7-7"])
}

pub fn log_out(class: &'static str) -> impl IntoView {
    icon("log-out", class, &["m16 17 5-5-5-5", "M21 12H9", "M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"])
}

pub fn loader_circle(class: &'static str) -> impl IntoView {
    icon("loader-circle", class, &["M21 12a9 9 0 1 1-6.219-8.56"])
}

pub fn rotate_ccw(class: &'static str) -> impl IntoView {
    icon("rotate-ccw", class, &["M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8", "M3 3v5h5"])
}

pub fn download(class: &'static str) -> impl IntoView {
    icon("download", class, &["M12 15V3", "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4", "m7 10 5 5 5-5"])
}

pub fn upload(class: &'static str) -> impl IntoView {
    icon("upload", class, &["M12 3v12", "m17 8-5-5-5 5", "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"])
}

pub fn triangle_alert(class: &'static str) -> impl IntoView {
    icon("triangle-alert", class, &["m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3", "M12 9v4", "M12 17h.01"])
}

pub fn play(class: &'static str) -> impl IntoView {
    icon("play", class, &["M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z"])
}

/// Bestandteil eines Symbols mit Kreisen oder Rechtecken.
enum Shape {
    Path(&'static str),
    Circle(&'static str, &'static str, &'static str),
    /// x, y, Breite, Höhe, Rundung
    Rect(&'static str, &'static str, &'static str, &'static str, &'static str),
}

fn shape_icon(name: &'static str, class: &'static str, shapes: &'static [Shape]) -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
            stroke-linecap="round" stroke-linejoin="round" class=format!("lucide lucide-{name} {class}") aria-hidden="true">
            {shapes.iter().map(|shape| match *shape {
                Shape::Path(d) => view! { <path d=d></path> }.into_any(),
                Shape::Circle(cx, cy, r) => view! { <circle cx=cx cy=cy r=r></circle> }.into_any(),
                Shape::Rect(x, y, width, height, rx) => view! { <rect x=x y=y width=width height=height rx=rx></rect> }.into_any(),
            }).collect_view()}
        </svg>
    }
}

pub fn message_square_reply(class: &'static str) -> impl IntoView {
    icon("message-square-reply", class, &["M22 17a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 21.286V5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2z", "m10 8-3 3 3 3", "M17 14v-1a2 2 0 0 0-2-2H7"])
}

pub fn pin(class: &'static str) -> impl IntoView {
    icon("pin", class, &["M12 17v5", "M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"])
}

pub fn pin_off(class: &'static str) -> impl IntoView {
    icon("pin-off", class, &["M12 17v5", "M15 9.34V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H7.89", "m2 2 20 20", "M9 9v1.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h11"])
}

pub fn languages(class: &'static str) -> impl IntoView {
    icon("languages", class, &["m5 8 6 6", "m4 14 6-6 2-3", "M2 5h12", "M7 2h1", "m22 22-5-10-5 10", "M14 18h6"])
}

pub fn chevron_right(class: &'static str) -> impl IntoView {
    icon("chevron-right", class, &["m9 18 6-6-6-6"])
}

pub fn ban(class: &'static str) -> impl IntoView {
    shape_icon("ban", class, &[Shape::Circle("12", "12", "10"), Shape::Path("M4.929 4.929 19.07 19.071")])
}

pub fn circle_check(class: &'static str) -> impl IntoView {
    shape_icon("circle-check", class, &[Shape::Circle("12", "12", "10"), Shape::Path("m16 9-5.5 5.5L8 12")])
}

pub fn octagon_x(class: &'static str) -> impl IntoView {
    icon("octagon-x", class, &["m15 9-6 6", "M2.586 16.726A2 2 0 0 1 2 15.312V8.688a2 2 0 0 1 .586-1.414l4.688-4.688A2 2 0 0 1 8.688 2h6.624a2 2 0 0 1 1.414.586l4.688 4.688A2 2 0 0 1 22 8.688v6.624a2 2 0 0 1-.586 1.414l-4.688 4.688a2 2 0 0 1-1.414.586H8.688a2 2 0 0 1-1.414-.586z", "m9 9 6 6"])
}

pub fn pause(class: &'static str) -> impl IntoView {
    shape_icon("pause", class, &[Shape::Rect("14", "3", "5", "18", "1"), Shape::Rect("5", "3", "5", "18", "1")])
}

pub fn copy(class: &'static str) -> impl IntoView {
    shape_icon("copy", class, &[Shape::Rect("8", "8", "14", "14", "2"), Shape::Path("M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2")])
}

pub fn search(class: &'static str) -> impl IntoView {
    shape_icon("search", class, &[Shape::Path("m21 21-4.34-4.34"), Shape::Circle("11", "11", "8")])
}

pub fn external_link(class: &'static str) -> impl IntoView {
    icon("external-link", class, &["M15 3h6v6", "M10 14 21 3", "M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"])
}

pub fn bell(class: &'static str) -> impl IntoView {
    icon("bell", class, &["M10.268 21a2 2 0 0 0 3.464 0", "M3.262 15.326A1 1 0 0 0 4 17h16a1 1 0 0 0 .74-1.673C19.41 13.956 18 12.499 18 8A6 6 0 0 0 6 8c0 4.499-1.411 5.956-2.738 7.326"])
}

pub fn bell_off(class: &'static str) -> impl IntoView {
    icon("bell-off", class, &["M10.268 21a2 2 0 0 0 3.464 0", "M17 17H4a1 1 0 0 1-.74-1.673C4.59 13.956 6 12.499 6 8a6 6 0 0 1 .258-1.742", "m2 2 20 20", "M8.668 3.01A6 6 0 0 1 18 8c0 2.687.77 4.653 1.707 6.05"])
}

pub fn ellipsis_vertical(class: &'static str) -> impl IntoView {
    shape_icon("ellipsis-vertical", class, &[Shape::Circle("12", "12", "1"), Shape::Circle("12", "5", "1"), Shape::Circle("12", "19", "1")])
}

pub fn radio(class: &'static str) -> impl IntoView {
    shape_icon("radio", class, &[Shape::Path("M16.247 7.761a6 6 0 0 1 0 8.478"), Shape::Path("M19.075 4.933a10 10 0 0 1 0 14.134"), Shape::Path("M4.925 19.067a10 10 0 0 1 0-14.134"), Shape::Path("M7.753 16.239a6 6 0 0 1 0-8.478"), Shape::Circle("12", "12", "2")])
}

pub fn refresh_cw(class: &'static str) -> impl IntoView {
    icon("refresh-cw", class, &["M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8", "M21 3v5h-5", "M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16", "M8 16H3v5"])
}

pub fn replace(class: &'static str) -> impl IntoView {
    shape_icon("replace", class, &[Shape::Path("M14 4a1 1 0 0 1 1-1"), Shape::Path("M15 10a1 1 0 0 1-1-1"), Shape::Path("M21 4a1 1 0 0 0-1-1"), Shape::Path("M21 9a1 1 0 0 1-1 1"), Shape::Path("m3 7 3 3 3-3"), Shape::Path("M6 10V5a2 2 0 0 1 2-2h2"), Shape::Rect("3", "14", "7", "7", "1")])
}

pub fn shield(class: &'static str) -> impl IntoView {
    icon("shield", class, &["M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"])
}

pub fn sliders_horizontal(class: &'static str) -> impl IntoView {
    icon("sliders-horizontal", class, &["M10 5H3", "M12 19H3", "M14 3v4", "M16 17v4", "M21 12h-9", "M21 19h-5", "M21 5h-7", "M8 10v4", "M8 12H3"])
}

pub fn sword(class: &'static str) -> impl IntoView {
    icon("sword", class, &["m11 19-6-6", "m5 21-2-2", "m8 16-4 4", "M9.5 17.5 20.414 6.586A2 2 0 0021 5.172V3h-2.172a2 2 0 00-1.414.586L6.5 14.5"])
}

pub fn tv(class: &'static str) -> impl IntoView {
    shape_icon("tv", class, &[Shape::Path("m17 2-5 5-5-5"), Shape::Rect("2", "7", "20", "15", "2")])
}

pub fn eye(class: &'static str) -> impl IntoView {
    shape_icon("eye", class, &[Shape::Path("M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0"), Shape::Circle("12", "12", "3")])
}

pub fn pencil(class: &'static str) -> impl IntoView {
    icon("pencil", class, &["M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z", "m15 5 4 4"])
}

pub fn app_window(class: &'static str) -> impl IntoView {
    shape_icon("app-window", class, &[Shape::Rect("2", "4", "20", "16", "2"), Shape::Path("M10 4v4"), Shape::Path("M2 8h20"), Shape::Path("M6 4v4")])
}

pub fn lock(class: &'static str) -> impl IntoView {
    shape_icon("lock", class, &[Shape::Rect("3", "11", "18", "11", "2"), Shape::Path("M7 11V7a5 5 0 0 1 10 0v4")])
}

pub fn lock_open(class: &'static str) -> impl IntoView {
    shape_icon("lock-open", class, &[Shape::Rect("3", "11", "18", "11", "2"), Shape::Path("M7 11V7a5 5 0 0 1 9.9-1")])
}

pub fn settings(class: &'static str) -> impl IntoView {
    shape_icon("settings", class, &[Shape::Path("M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915"), Shape::Circle("12", "12", "3")])
}

pub fn info(class: &'static str) -> impl IntoView {
    shape_icon("info", class, &[Shape::Circle("12", "12", "10"), Shape::Path("M12 16v-4"), Shape::Path("M12 8h.01")])
}

pub fn grip_vertical(class: &'static str) -> impl IntoView {
    shape_icon(
        "grip-vertical",
        class,
        &[
            Shape::Circle("9", "12", "1"),
            Shape::Circle("9", "5", "1"),
            Shape::Circle("9", "19", "1"),
            Shape::Circle("15", "12", "1"),
            Shape::Circle("15", "5", "1"),
            Shape::Circle("15", "19", "1"),
        ],
    )
}

pub fn video(class: &'static str) -> impl IntoView {
    shape_icon("video", class, &[Shape::Path("m16 13 5.223 3.482a.5.5 0 0 0 .777-.416V7.87a.5.5 0 0 0-.752-.432L16 10.5"), Shape::Rect("2", "6", "14", "12", "2")])
}

pub fn wrench(class: &'static str) -> impl IntoView {
    icon("wrench", class, &["M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"])
}

pub fn star(class: &'static str) -> impl IntoView {
    icon("star", class, &["M11.525 2.295a.53.53 0 0 1 .95 0l2.31 4.679a2.123 2.123 0 0 0 1.595 1.16l5.166.756a.53.53 0 0 1 .294.904l-3.736 3.638a2.123 2.123 0 0 0-.611 1.878l.882 5.14a.53.53 0 0 1-.771.56l-4.618-2.428a2.122 2.122 0 0 0-1.973 0L6.396 21.01a.53.53 0 0 1-.77-.56l.881-5.139a2.122 2.122 0 0 0-.611-1.879L2.16 9.795a.53.53 0 0 1 .294-.906l5.165-.755a2.122 2.122 0 0 0 1.597-1.16z"])
}

pub fn badge_check(class: &'static str) -> impl IntoView {
    icon("badge-check", class, &["M3.85 8.62a4 4 0 0 1 4.78-4.77 4 4 0 0 1 6.74 0 4 4 0 0 1 4.78 4.78 4 4 0 0 1 0 6.74 4 4 0 0 1-4.77 4.78 4 4 0 0 1-6.75 0 4 4 0 0 1-4.78-4.77 4 4 0 0 1 0-6.76Z", "m9 12 2 2 4-4"])
}

/// Plattform-Logo (Pfade aus Simple Icons, CC0), gefüllt in der Textfarbe. Kennzeichnet Twitch- und YouTube-Tabs.
fn brand_icon(name: &'static str, class: &'static str, label: &'static str, path: &'static str) -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"
            class=format!("brand-{name} {class}") role="img" aria-label=label>
            <title>{label}</title>
            <path d=path></path>
        </svg>
    }
}

pub fn twitch_logo(class: &'static str) -> impl IntoView {
    brand_icon("twitch", class, "Twitch", "M11.571 4.714h1.715v5.143H11.57zm4.715 0H18v5.143h-1.714zM6 0L1.714 4.286v15.428h5.143V24l4.286-4.286h3.428L22.286 12V0zm14.571 11.143l-3.428 3.428h-3.429l-3 3v-3H6.857V1.714h13.714Z")
}

pub fn youtube_logo(class: &'static str) -> impl IntoView {
    brand_icon("youtube", class, "YouTube", "M23.498 6.186a3.016 3.016 0 0 0-2.122-2.136C19.505 3.545 12 3.545 12 3.545s-7.505 0-9.377.505A3.017 3.017 0 0 0 .502 6.186C0 8.07 0 12 0 12s0 3.93.502 5.814a3.016 3.016 0 0 0 2.122 2.136c1.871.505 9.376.505 9.376.505s7.505 0 9.377-.505a3.015 3.015 0 0 0 2.122-2.136C24 15.93 24 12 24 12s0-3.93-.502-5.814zM9.545 15.568V8.432L15.818 12l-6.273 3.568z")
}
