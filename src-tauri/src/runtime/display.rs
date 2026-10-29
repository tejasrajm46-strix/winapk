//! Where an app window's pixels come from.
//!
//! One policy decides two things together, because they only work as a pair: the
//! Android virtual display an app is drawn on, and the Windows window that shows
//! it. DroidWin used to create every display portrait at 540x1170/420 — 205x445
//! dp, less than half a phone — and then ask for a portrait window. A
//! landscape-locked game (Block'Buster) rotated that surface to 1170x540 and drew
//! a layout taller than 205 dp, so its own content was clipped by the display,
//! and the portrait window showed a landscape stream in a fraction of its area.
//!
//! Geometry is therefore chosen in dp, not pixels: a display is a phone surface
//! (411x914 dp at the emulator's own 420 dpi). A landscape app gets the same
//! surface rotated, which is exactly what it would get on a phone turned
//! sideways. The window keeps that aspect ratio, so the window host has nothing
//! to letterbox.
//!
//! These numbers are the single source of truth. `EmulatorRuntime` records the
//! geometry it created for each open window; input mapping, screenshot
//! conversion and the control API read it back instead of repeating constants.

use serde::Serialize;

use crate::domain::AppOrientation;

/// Density of every DroidWin display. It is the emulator's own screen density,
/// so an app selects the same assets and sees the same dp space it would on the
/// device itself.
pub const DENSITY_DPI: u32 = 420;

/// The device's own display — Android's launcher, its own notifications and its
/// own settings. Every other display DroidWin touches it created itself, so this
/// is the one id that is not DroidWin's to choose. The virtual phone screen
/// mirrors it and sends its touches to it, which is why it is named here rather
/// than repeated as a literal.
pub const DEVICE_DISPLAY: u32 = 0;

/// Phone surface in dp — what a Pixel-class display reports at 420 dpi
/// (1080x2400 px portrait, 2400x1080 px landscape).
const SURFACE_WIDTH_DP: u32 = 411;
const SURFACE_HEIGHT_DP: u32 = 914;

/// Window size relative to its display. One ratio for both orientations: the
/// window shows the whole surface, scaled, with no letterboxing.
const WINDOW_SCALE_NUMERATOR: u32 = 7;
const WINDOW_SCALE_DENOMINATOR: u32 = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    Portrait,
    Landscape,
}

impl Orientation {
    pub fn of(app: AppOrientation) -> Self {
        match app {
            AppOrientation::Portrait => Orientation::Portrait,
            AppOrientation::Landscape => Orientation::Landscape,
        }
    }

    /// The orientation a composed size is drawn in. A square (or empty) size
    /// says nothing, so it reports none instead of guessing.
    pub fn of_size(width: u32, height: u32) -> Option<Self> {
        match width.cmp(&height) {
            std::cmp::Ordering::Greater => Some(Orientation::Landscape),
            std::cmp::Ordering::Less => Some(Orientation::Portrait),
            std::cmp::Ordering::Equal => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Orientation::Portrait => "portrait",
            Orientation::Landscape => "landscape",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "portrait" => Some(Orientation::Portrait),
            "landscape" => Some(Orientation::Landscape),
            _ => None,
        }
    }

    pub fn flipped(self) -> Self {
        match self {
            Orientation::Portrait => Orientation::Landscape,
            Orientation::Landscape => Orientation::Portrait,
        }
    }

    /// Where a user-chosen orientation for one app is remembered. A virtual
    /// display is created once, so changing it means opening the window again;
    /// the choice has to outlive the close.
    pub fn setting_key(package_id: &str) -> String {
        format!("app.orientation.{package_id}")
    }
}

/// The Android surface an app window is drawn on, and the Windows window that
/// shows it. Both sizes are in pixels; `dpi` is the display density.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DisplayGeometry {
    pub orientation: Orientation,
    pub width: u32,
    pub height: u32,
    pub dpi: u32,
    pub window_width: u32,
    pub window_height: u32,
}

impl DisplayGeometry {
    pub fn portrait() -> Self {
        Self::for_orientation(Orientation::Portrait)
    }

    pub fn landscape() -> Self {
        Self::for_orientation(Orientation::Landscape)
    }

    /// Geometry for an app that declared an orientation. `None` — the manifest
    /// does not restrict it — takes the portrait surface, which every Android app
    /// supports; the runtime records what Android did and uses it next time.
    pub fn for_app(app: Option<AppOrientation>) -> Self {
        match app {
            Some(app) => Self::for_orientation(Orientation::of(app)),
            None => Self::portrait(),
        }
    }

    pub fn for_orientation(orientation: Orientation) -> Self {
        let (width_dp, height_dp) = match orientation {
            Orientation::Portrait => (SURFACE_WIDTH_DP, SURFACE_HEIGHT_DP),
            Orientation::Landscape => (SURFACE_HEIGHT_DP, SURFACE_WIDTH_DP),
        };
        let width = pixels(width_dp);
        let height = pixels(height_dp);
        Self {
            orientation,
            width,
            height,
            dpi: DENSITY_DPI,
            window_width: window_extent(width),
            window_height: window_extent(height),
        }
    }
}

/// Width and height from a PNG's IHDR chunk. A capture is the only place window
/// and display sizes are read back from Android, so the reader lives with the
/// geometry rather than beside any one caller.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || !png.starts_with(b"\x89PNG") {
        return None;
    }
    let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    Some((width, height))
}

/// dp → px at [`DENSITY_DPI`], rounded up to an even number because the video
/// encoder behind the window wants even dimensions.
fn pixels(dp: u32) -> u32 {
    let px = (dp * DENSITY_DPI + 80) / 160;
    px + (px & 1)
}

fn window_extent(display_px: u32) -> u32 {
    (display_px * WINDOW_SCALE_NUMERATOR + WINDOW_SCALE_DENOMINATOR / 2) / WINDOW_SCALE_DENOMINATOR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phone_surface_is_the_emulators_own_size() {
        let portrait = DisplayGeometry::portrait();
        assert_eq!((portrait.width, portrait.height), (1080, 2400));
        assert_eq!(portrait.dpi, 420);
        let landscape = DisplayGeometry::landscape();
        assert_eq!((landscape.width, landscape.height), (2400, 1080));
        assert_eq!(landscape.dpi, 420);
    }

    #[test]
    fn a_chosen_orientation_flips_and_round_trips_through_its_setting() {
        assert_eq!(Orientation::Portrait.flipped(), Orientation::Landscape);
        assert_eq!(Orientation::Landscape.flipped(), Orientation::Portrait);
        assert_eq!(Orientation::parse(" Landscape "), Some(Orientation::Landscape));
        assert_eq!(Orientation::parse("sideways"), None);
        assert_eq!(
            Orientation::setting_key("com.example.game"),
            "app.orientation.com.example.game"
        );
        assert_eq!(
            DisplayGeometry::for_orientation(Orientation::Landscape).orientation,
            Orientation::Landscape
        );
    }

    #[test]
    fn the_surface_is_measured_in_dp_not_pixels() {
        // 411x914 dp at 420 dpi. The old 540x1170 surface was 205x445 dp, which is
        // why a landscape layout taller than 205 dp was clipped by its own display.
        for geometry in [DisplayGeometry::portrait(), DisplayGeometry::landscape()] {
            let longer = geometry.width.max(geometry.height) as f64 * 160.0 / geometry.dpi as f64;
            let shorter = geometry.width.min(geometry.height) as f64 * 160.0 / geometry.dpi as f64;
            assert!((longer - 914.0).abs() <= 2.0, "long edge {longer} dp");
            assert!((shorter - 411.0).abs() <= 2.0, "short edge {shorter} dp");
        }
    }

    #[test]
    fn the_window_has_the_displays_aspect_ratio() {
        for geometry in [DisplayGeometry::portrait(), DisplayGeometry::landscape()] {
            // Cross-multiplied ratios: width/height must match. Whole-pixel
            // rounding may leave a sub-pixel difference, so the tolerance is one
            // display pixel, not exact equality.
            let left = geometry.width as i64 * geometry.window_height as i64;
            let right = geometry.window_width as i64 * geometry.height as i64;
            let tolerance = geometry.width.max(geometry.height) as i64;
            assert!(
                (left - right).abs() <= tolerance,
                "{geometry:?} keeps the aspect ratio ({left} vs {right})"
            );
            assert!(geometry.window_width < geometry.width);
            assert!(geometry.window_height < geometry.height);
        }
    }

    #[test]
    fn a_declared_orientation_selects_its_surface() {
        let landscape = DisplayGeometry::for_app(Some(AppOrientation::Landscape));
        assert_eq!(landscape.orientation, Orientation::Landscape);
        assert_eq!(
            (landscape.window_width, landscape.window_height),
            (933, 420)
        );

        let portrait = DisplayGeometry::for_app(Some(AppOrientation::Portrait));
        assert_eq!(portrait.orientation, Orientation::Portrait);
        assert_eq!((portrait.window_width, portrait.window_height), (420, 933));
    }

    #[test]
    fn an_undeclared_orientation_falls_back_to_portrait_without_claiming_more() {
        let geometry = DisplayGeometry::for_app(None);
        assert_eq!(geometry.orientation, Orientation::Portrait);
        assert_eq!((geometry.width, geometry.height), (1080, 2400));
    }

    #[test]
    fn a_composed_size_reports_the_orientation_it_is_drawn_in() {
        // A portrait surface Android rotated for a landscape app composes 1170x540.
        assert_eq!(
            Orientation::of_size(1170, 540),
            Some(Orientation::Landscape)
        );
        assert_eq!(Orientation::of_size(540, 1170), Some(Orientation::Portrait));
        assert_eq!(Orientation::of_size(540, 540), None);
        assert_eq!(Orientation::of_size(0, 0), None);
    }

    #[test]
    fn stored_orientation_text_round_trips() {
        assert_eq!(
            AppOrientation::parse("Landscape"),
            Some(AppOrientation::Landscape)
        );
        assert_eq!(
            AppOrientation::parse(AppOrientation::Portrait.as_str()),
            Some(AppOrientation::Portrait)
        );
        assert_eq!(AppOrientation::parse("sensor"), None);
    }

    #[test]
    fn a_captures_size_comes_from_its_own_header() {
        let mut png = vec![0x89, b'P', b'N', b'G'];
        png.resize(16, 0);
        png.extend_from_slice(&540u32.to_be_bytes());
        png.extend_from_slice(&1170u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((540, 1170)));
        assert_eq!(png_size(b"not a png at all, definitely not"), None);
        assert_eq!(png_size(&[0x89, b'P', b'N', b'G']), None);
    }
}
