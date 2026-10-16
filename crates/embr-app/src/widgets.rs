//! Small shared pieces of the window, in one place so that a decision is made once.

use adw::prelude::*;

use embr_core::{DeviceView, LOW_AT_OR_BELOW, Level, MEDIUM_AT_OR_BELOW, level_color};

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
    for class in LEVEL_CLASSES {
        bar.remove_css_class(class);
    }
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
    if let Some(class) = class {
        bar.add_css_class(class);
    }
    bar.set_tooltip_text(tooltip);
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
