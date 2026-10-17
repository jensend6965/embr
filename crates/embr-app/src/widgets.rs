//! Small shared pieces of the window, in one place so that a decision is made once.

use adw::prelude::*;

use embr_core::{DeviceKind, DeviceView, LOW_AT_OR_BELOW, Level, MEDIUM_AT_OR_BELOW, level_color};

/// A row whose title and subtitle are text, not markup.
///
/// `AdwActionRow` parses both as Pango markup by default, so the label `DPI & sensitivity` — which
/// is what `core` calls that page — silently renders as *nothing* (measured: `Failed to set text
/// … due to error parsing markup`), and so does any device name with an `&` or a `<` in it.
/// Everything this window prints comes from `core` as plain text, so it says so rather than
/// escaping our own wording to suit one toolkit.
pub fn row(title: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(title)
        .use_markup(false)
        .build()
}

/// Text for a widget that parses Pango markup, with the markup escaped.
///
/// Adwaita's label-bearing setters — `AdwPreferencesGroup::description`,
/// `AdwActionRow::subtitle` — parse Pango markup, and our wording is ordinary text that may contain
/// `<`, `>` or `&`: `embr-<device>` and `DPI & sensitivity` both do, and the first one came out of
/// the running app as `Error on line 1 char 170: Element “markup” was closed, but the currently open
/// element is “device”` with the sentence half-drawn. Escaping happens here, at the widget
/// boundary — the wording stays ordinary text in `core`, which is where it is decided.
///
/// A preference *group* has no `use-markup` property to turn off, which is what makes this
/// necessary beside `row` rather than instead of it.
pub fn escaped(text: &str) -> String {
    gtk4::glib::markup_escape_text(text).to_string()
}

/// The same, for a row that knows its subtitle as it is built.
pub fn row_with(title: &str, subtitle: &str) -> adw::ActionRow {
    let row = row(title);
    row.set_subtitle(&escaped(subtitle));
    row
}

/// The three classes a level bar's fill can be drawn with. Ours, not the theme's: see `level_css`.
const LEVEL_CLASSES: [&str; 3] = ["embr-level-low", "embr-level-middle", "embr-level-full"];

/// The colours a level bar's fill is drawn in, as CSS, **from the one palette in `core`**.
///
/// Which node is "the fill" is the theme's own answer: this desktop's stylesheet paints
/// `levelbar > trough > block:not(.empty)` with the accent colour and `block.empty` with a
/// currentColor mix, and GTK's older spelling for the same block is `block.filled`. Both are in the
/// selector, so the reading's colour lands on the filled part and the empty part keeps the trough's.
/// `background-image: none` because a theme is as likely to have set a gradient as a flat colour.
///
/// Not through libadwaita's `.success`/`.warning`/`.error`: those are the theme's to define, and
/// this desktop's (`adw-gtk3-dark`) colours nothing with them that the theme did not intend — a
/// percentage label came out in the ordinary text colour, which is how this was found. The numbers
/// come from `core::level_color`, the function the tray icon's `level` text colour uses, so the two
/// surfaces cannot drift apart.
///
/// Priority `APPLICATION` beats the theme's stylesheet outright: provider priority is compared
/// *before* specificity, so this wins over `levelbar > trough > block:not(.empty)` and over
/// `block.low`/`block.full` without a selector fight.
pub fn level_css() -> String {
    let rule = |class: &str, level: u8| {
        let (r, g, b) = level_color(Some(level));
        format!(
            "levelbar.{class} > trough > block:not(.empty),\n\
             levelbar.{class} > trough > block.filled {{ background-color: rgb({r},{g},{b}); \
             background-image: none; }}\n"
        )
    };
    let mut css = String::new();
    css.push_str(&rule(LEVEL_CLASSES[0], LOW_AT_OR_BELOW));
    css.push_str(&rule(LEVEL_CLASSES[1], MEDIUM_AT_OR_BELOW));
    // 100 is always a full reading, whatever the middle edge is set to.
    css.push_str(&rule(LEVEL_CLASSES[2], 100));
    css
}

/// Install [`level_css`] once, on the display the window will be drawn on.
pub fn install_level_css() {
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(&level_css());
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Write a label only when it would change.
///
/// This is for the per-tick path: four times a second, a write that says the same thing still costs
/// a relayout, and a relayout under the pointer is what "choppy" feels like.
pub fn set_text(label: &gtk4::Label, text: &str) {
    if label.text() != text {
        label.set_label(text);
    }
}

/// The same, for a row's subtitle — which parses markup, so the text is escaped on the way in.
pub fn set_subtitle(row: &adw::ActionRow, text: &str) {
    if row.subtitle().as_deref() != Some(text) {
        row.set_subtitle(&escaped(text));
    }
}

/// The same, for a row's title.
pub fn set_title(row: &adw::ActionRow, text: &str) {
    if row.title().as_str() != text {
        row.set_title(text);
    }
}

/// Draw a device's level bar in the colour of its level.
///
/// `core` decides which band a reading is in and what that band's colour is; this only puts the
/// class on the bar. The *percentage beside it keeps the ordinary text colour*: the bar is the
/// picture, the number is the reading, and colouring both said the same thing twice.
///
/// A reading with no band is a bar in the theme's accent colour, which is also what a user who
/// switched the colouring off gets — a device with nothing to read is not a warning.
///
/// A device reporting a fault is red whatever its level: there the number is not the problem, and a
/// fault on a full battery must not read as healthy.
pub fn paint_level(bar: &gtk4::LevelBar, device: &DeviceView) {
    let (class, tooltip) = if device.faulty {
        (
            Some(LEVEL_CLASSES[0]),
            Some("this device reports a battery fault"),
        )
    } else {
        match device.level {
            Some(Level::Low) => (Some(LEVEL_CLASSES[0]), Some("this battery is low")),
            Some(Level::Middle) => (Some(LEVEL_CLASSES[1]), Some("this battery is getting low")),
            Some(Level::Full) => (Some(LEVEL_CLASSES[2]), None),
            None => (None, None),
        }
    };
    // Only when it differs: adding and removing a style class invalidates the widget's CSS every
    // time, and this is called with every tick for every device.
    let current = LEVEL_CLASSES
        .iter()
        .find(|candidate| bar.has_css_class(candidate))
        .copied();
    if current != class {
        for candidate in LEVEL_CLASSES {
            bar.remove_css_class(candidate);
        }
        if let Some(class) = class {
            bar.add_css_class(class);
        }
    }
    if bar.tooltip_text().as_deref() != tooltip {
        bar.set_tooltip_text(tooltip);
    }
}
/// The widget a device glyph is drawn in, in logical pixels — big enough to read beside a name,
/// which the first attempt at 18 was not.
const GLYPH: i32 = 26;

/// The coordinate box the shapes are described in. Separate from [`GLYPH`] so the drawing can be
/// made bigger without re-deriving every number: the context is scaled from this to the widget.
const DESIGN: f64 = 18.0;

/// Every kind a device can be, in a fixed order — what the glyph probe draws.
pub const DEVICE_KINDS: [DeviceKind; 9] = [
    DeviceKind::Mouse,
    DeviceKind::Keyboard,
    DeviceKind::Headset,
    DeviceKind::Trackball,
    DeviceKind::Touchpad,
    DeviceKind::Numpad,
    DeviceKind::Presenter,
    DeviceKind::Remote,
    DeviceKind::Unknown,
];

/// A device's kind, drawn from primitives instead of looked up as an icon name.
///
/// An icon *name* is a bet on the desktop's icon theme, and this desktop loses that bet: GTK
/// resolves its theme as `"gnome"`, answers `has_icon` for only a handful of symbolic names, and
/// draws the rest as its missing-image box — the same finding that took the icons off the feature
/// tabs. Shapes cannot be missing: they are identical on every desktop and every icon theme, and
/// they take the row's own colour, so they dim with a row that is not answering and follow light,
/// dark and the accent without a second set of assets.
pub fn device_glyph(kind: DeviceKind) -> gtk4::DrawingArea {
    let area = gtk4::DrawingArea::new();
    area.set_content_width(GLYPH);
    area.set_content_height(GLYPH);
    area.set_valign(gtk4::Align::Center);
    // An icon beside a title is secondary information, and this is the class the desktop already
    // uses for that, so it fades correctly in either scheme.
    area.add_css_class("dim-label");
    area.set_draw_func(move |area, cr, width, height| {
        let colour = area.color();
        cr.set_source_rgba(
            f64::from(colour.red()),
            f64::from(colour.green()),
            f64::from(colour.blue()),
            f64::from(colour.alpha()),
        );
        draw_glyph(cr, kind, width, height);
    });
    area
}

/// Draw one kind into `cr`, scaled from the 18-unit box the shapes are described in.
///
/// The colour is the caller's, so `EMBR_DEBUG_GLYPHS` can render the whole set to a PNG for review
/// with no window, no compositor and no icon theme in the way.
pub fn draw_glyph(cr: &gtk4::cairo::Context, kind: DeviceKind, width: i32, height: i32) {
    cr.scale(f64::from(width) / DESIGN, f64::from(height) / DESIGN);
    cr.set_line_width(1.4);
    cr.set_line_cap(gtk4::cairo::LineCap::Round);
    cr.set_line_join(gtk4::cairo::LineJoin::Round);

    match kind {
        DeviceKind::Mouse => {
            rounded_rect(cr, 5.5, 2.5, 7.0, 13.0, 3.2);
            // The button split, which is what makes the body read as a mouse and not a pill.
            cr.move_to(9.0, 2.9);
            cr.line_to(9.0, 7.2);
        }
        DeviceKind::Keyboard => {
            rounded_rect(cr, 2.0, 5.5, 14.0, 7.0, 1.6);
            for x in [5.2, 8.2, 11.2] {
                cr.move_to(x, 9.9);
                cr.line_to(x + 1.4, 9.9);
            }
        }
        DeviceKind::Headset => {
            // The band over the top, then the two cups it joins.
            cr.arc(9.0, 10.4, 6.2, std::f64::consts::PI, std::f64::consts::TAU);
            rounded_rect(cr, 1.6, 9.0, 3.6, 5.6, 1.6);
            rounded_rect(cr, 12.8, 9.0, 3.6, 5.6, 1.6);
        }
        DeviceKind::Trackball => {
            // A ball sitting in a base: the outer ring, the ball above the chord, the base below it.
            cr.arc(9.0, 9.0, 6.2, 0.0, std::f64::consts::TAU);
            cr.move_to(4.0, 11.8);
            cr.line_to(14.0, 11.8);
            cr.arc(9.0, 7.4, 2.0, 0.0, std::f64::consts::TAU);
        }
        DeviceKind::Touchpad => {
            rounded_rect(cr, 2.0, 3.5, 14.0, 11.0, 1.6);
            cr.move_to(6.5, 12.8);
            cr.line_to(11.5, 12.8);
        }
        DeviceKind::Numpad => {
            rounded_rect(cr, 4.5, 2.5, 9.0, 13.0, 1.6);
            for y in [6.5, 9.5, 12.0] {
                for x in [7.2, 10.6] {
                    cr.move_to(x, y);
                    cr.line_to(x + 0.01, y);
                }
            }
        }
        DeviceKind::Presenter | DeviceKind::Remote => {
            rounded_rect(cr, 5.5, 2.5, 7.0, 13.0, 3.2);
            for y in [7.0, 10.0] {
                cr.move_to(9.0, y);
                cr.line_to(9.01, y);
            }
        }
        DeviceKind::Unknown => {
            // Two rings: a device Embr can see but cannot name.
            cr.arc(9.0, 9.0, 6.2, 0.0, std::f64::consts::TAU);
            cr.arc(9.0, 9.0, 2.6, 0.0, std::f64::consts::TAU);
        }
    }
    let _ = cr.stroke();
}

/// A rounded rectangle as a cairo path: four straight sides and four quarter-circle corners.
fn rounded_rect(cr: &gtk4::cairo::Context, x: f64, y: f64, width: f64, height: f64, radius: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let radius = radius.min(width / 2.0).min(height / 2.0);
    let (right, bottom) = (x + width, y + height);
    cr.move_to(x + radius, y);
    cr.line_to(right - radius, y);
    cr.arc(right - radius, y + radius, radius, -FRAC_PI_2, 0.0);
    cr.line_to(right, bottom - radius);
    cr.arc(right - radius, bottom - radius, radius, 0.0, FRAC_PI_2);
    cr.line_to(x + radius, bottom);
    cr.arc(x + radius, bottom - radius, radius, FRAC_PI_2, PI);
    cr.line_to(x, y + radius);
    cr.arc(x + radius, y + radius, radius, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

/// Render every device glyph into one PNG and say where it went.
///
/// The counterpart to `EMBR_DEBUG_GLYPHS`: a glyph is geometry, so it can be reviewed offscreen,
/// at an exact size, with no window, compositor or icon theme involved.
pub fn write_glyph_sheet() {
    let cell = 32;
    let count = DEVICE_KINDS.len() as i32;
    let Ok(mut surface) =
        gtk4::cairo::ImageSurface::create(gtk4::cairo::Format::ARgb32, cell * count, cell)
    else {
        eprintln!("[glyphs] could not make a surface");
        return;
    };
    let Ok(cr) = gtk4::cairo::Context::new(&surface) else {
        eprintln!("[glyphs] could not make a context");
        return;
    };
    // The window's own dark surface, so the review looks like the rail does.
    cr.set_source_rgb(0.11, 0.12, 0.14);
    let _ = cr.paint();
    for (index, kind) in DEVICE_KINDS.iter().enumerate() {
        cr.save().ok();
        cr.translate(
            f64::from(cell * index as i32 + (cell - GLYPH) / 2),
            f64::from((cell - GLYPH) / 2),
        );
        cr.set_source_rgb(0.93, 0.94, 0.95);
        draw_glyph(&cr, *kind, GLYPH, GLYPH);
        cr.restore().ok();
    }
    // The context holds a borrow on the surface: let it go before reading the pixels back.
    drop(cr);
    surface.flush();
    // A PPM, because cairo's PNG writer is behind a crate feature this build does not enable and a
    // six-line header is cheaper than a dependency for a debug switch.
    let (width, height) = (surface.width(), surface.height());
    let stride = surface.stride() as usize;
    let Ok(data) = surface.data() else {
        eprintln!("[glyphs] could not read the surface");
        return;
    };
    let mut ppm = format!("P6 {width} {height} 255\n").into_bytes();
    for row in 0..height as usize {
        for column in 0..width as usize {
            let at = row * stride + column * 4;
            let pixel = &data[at..at + 4];
            // ARGB32 is stored native-endian: BGRA on a little-endian machine, ARGB on a big one.
            let rgb = if cfg!(target_endian = "little") {
                [pixel[2], pixel[1], pixel[0]]
            } else {
                [pixel[1], pixel[2], pixel[3]]
            };
            ppm.extend_from_slice(&rgb);
        }
    }
    let path = std::env::temp_dir().join("embr-glyphs.ppm");
    match std::fs::write(&path, ppm) {
        Ok(()) => eprintln!("[glyphs] wrote {} ({} kinds)", path.display(), count),
        Err(err) => eprintln!("[glyphs] could not write {}: {err}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window draws `core`'s colours in the fill of a level bar — the same call the tray icon's
    /// level text colour makes. If this stops being true, a battery can be amber in the window and
    /// green in the tray.
    #[test]
    fn the_reading_colours_are_cores_own_palette_in_the_bars_fill() {
        let css = level_css();
        for (class, level) in [
            (LEVEL_CLASSES[0], LOW_AT_OR_BELOW),
            (LEVEL_CLASSES[1], MEDIUM_AT_OR_BELOW),
            (LEVEL_CLASSES[2], 100),
        ] {
            let (r, g, b) = level_color(Some(level));
            assert!(
                css.contains(&format!(
                    "levelbar.{class} > trough > block:not(.empty),\nlevelbar.{class} > trough > block.filled {{ background-color: rgb({r},{g},{b});"
                )),
                "{class} should fill with core's colour for {level}%: {css}"
            );
        }
        assert_eq!(
            css.matches("rgb(").count(),
            3,
            "three bands, three colours: {css}"
        );
        // "background-color: rgb(…)" contains "color: rgb", so the check is for a rule whose
        // *only* colour is a text colour.
        assert_eq!(
            css.matches("{ color: rgb").count(),
            0,
            "the fill is coloured, the text is not: {css}"
        );
    }

    /// Ordinary text goes into markup-parsing setters as it is written, and comes out as it is
    /// written: the two characters that broke the settings page.
    #[test]
    fn markup_in_our_wording_is_escaped_at_the_widget() {
        assert_eq!(escaped("DPI & sensitivity"), "DPI &amp; sensitivity");
        assert_eq!(escaped("embr-<device>"), "embr-&lt;device&gt;");
    }
}
