//! The phone display's size and density.
//!
//! These are Android's own `wm size` / `wm density` overrides on display 0 —
//! the display the virtual phone mirrors — not a Windows window setting. They
//! apply immediately and undo with `wm size reset`, so a resolution choice does
//! not cost an emulator restart. The numbers reported back are read from
//! Android, never assumed from what was requested.

use std::time::Duration;

use serde::Serialize;

use super::adb::Adb;
use crate::domain::{DwError, DwResult, ErrorCode};

/// Bounds that are certainly a real display and certainly not a typo: a 320px
/// phone is possible, a 32 000px one is not.
pub const MIN_EDGE: u32 = 320;
pub const MAX_EDGE: u32 = 3_840;
pub const MIN_DENSITY: u32 = 120;
pub const MAX_DENSITY: u32 = 640;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DisplayInfo {
    /// What Android reports as the display's own size.
    pub default_width: Option<u32>,
    pub default_height: Option<u32>,
    /// The size Android is currently using (the override, when one is set).
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub default_density: Option<u32>,
    pub density: Option<u32>,
    /// True when an override is in effect.
    pub overridden: bool,
}

/// A display size in device pixels.
pub type Size = (u32, u32);

fn split_size(line: &str) -> Option<Size> {
    let (width, height) = line.trim().split_once(['x', 'X'])?;
    let width = width.trim().parse::<u32>().ok()?;
    let height = height.trim().parse::<u32>().ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

/// `wm size` prints `Physical size: 1080x2400` and, with an override,
/// `Override size: 720x1600`.
pub fn parse_wm_size(text: &str) -> (Option<Size>, Option<Size>) {
    let mut physical = None;
    let mut overridden = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Physical size:") {
            physical = split_size(rest);
        } else if let Some(rest) = line.strip_prefix("Override size:") {
            overridden = split_size(rest);
        }
    }
    (physical, overridden)
}

/// `wm density` prints `Physical density: 420` and, with an override,
/// `Override density: 280`.
pub fn parse_wm_density(text: &str) -> (Option<u32>, Option<u32>) {
    let mut physical = None;
    let mut overridden = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Physical density:") {
            physical = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("Override density:") {
            overridden = rest.trim().parse().ok();
        }
    }
    (physical, overridden)
}

pub fn validate_size(width: u32, height: u32) -> Result<Size, String> {
    if !(MIN_EDGE..=MAX_EDGE).contains(&width) || !(MIN_EDGE..=MAX_EDGE).contains(&height) {
        return Err(format!(
            "The phone's display has to be between {MIN_EDGE} and {MAX_EDGE} pixels on each side; {width}×{height} is not."
        ));
    }
    Ok((width, height))
}

pub fn validate_density(density: u32) -> Result<u32, String> {
    if !(MIN_DENSITY..=MAX_DENSITY).contains(&density) {
        return Err(format!(
            "Android accepts densities from {MIN_DENSITY} to {MAX_DENSITY} dpi; {density} is not one of them."
        ));
    }
    Ok(density)
}

pub async fn read(adb: &Adb, serial: &str) -> DwResult<DisplayInfo> {
    let size = adb
        .run(
            &["-s", serial, "shell", "wm", "size"],
            Duration::from_secs(15),
        )
        .await?;
    let density = adb
        .run(
            &["-s", serial, "shell", "wm", "density"],
            Duration::from_secs(15),
        )
        .await?;
    let (physical, overridden) = parse_wm_size(&size.combined());
    let (physical_density, density_override) = parse_wm_density(&density.combined());
    let effective = overridden.or(physical);
    Ok(DisplayInfo {
        default_width: physical.map(|(w, _)| w),
        default_height: physical.map(|(_, h)| h),
        width: effective.map(|(w, _)| w),
        height: effective.map(|(_, h)| h),
        default_density: physical_density,
        density: density_override.or(physical_density),
        overridden: overridden.is_some() || density_override.is_some(),
    })
}

pub async fn apply(
    adb: &Adb,
    serial: &str,
    width: u32,
    height: u32,
    density: Option<u32>,
) -> DwResult<DisplayInfo> {
    let (width, height) = validate_size(width, height).map_err(|message| {
        DwError::new(ErrorCode::InvalidConfig, message)
            .detail("Choose a size the phone's display can actually use.")
    })?;
    let density = density
        .map(validate_density)
        .transpose()
        .map_err(|message| {
            DwError::new(ErrorCode::InvalidConfig, message)
                .detail("Choose a density Android can use, or leave it at the default.")
        })?;
    let out = adb
        .run(
            &["-s", serial, "shell", &format!("wm size {width}x{height}")],
            Duration::from_secs(20),
        )
        .await?;
    if !out.ok() {
        return Err(DwError::new(
            ErrorCode::BackendFailure,
            "Android refused that display size.",
        )
        .detail(out.combined()));
    }
    if let Some(density) = density {
        let out = adb
            .run(
                &["-s", serial, "shell", &format!("wm density {density}")],
                Duration::from_secs(20),
            )
            .await?;
        if !out.ok() {
            return Err(
                DwError::new(ErrorCode::BackendFailure, "Android refused that density.")
                    .detail(out.combined()),
            );
        }
    }
    // `wm` answers before the display has been reconfigured, and the capture
    // follows a moment later. Reporting the new size before Android composes at
    // it would make the phone's pixels and its touch space disagree for a
    // second, so the change is confirmed by what `screencap` actually returns.
    if !wait_for_capture(adb, serial, (width, height)).await {
        return Err(DwError::new(
            ErrorCode::BackendFailure,
            "Android accepted the display size but is still composing at the old one.",
        )
        .detail(format!(
            "No {width}x{height} capture appeared after 4 s. The size is set; the phone screen will follow it when Android recomposes."
        )));
    }
    read(adb, serial).await
}

/// Poll the capture's own header until it is the size Android just accepted.
/// Only 33 bytes are moved per probe, and only when a size is being changed.
async fn wait_for_capture(adb: &Adb, serial: &str, size: Size) -> bool {
    for _ in 0..10 {
        let header = adb
            .run_bytes(
                &[
                    "-s",
                    serial,
                    "exec-out",
                    "sh -c 'screencap -p | head -c 33'",
                ],
                Duration::from_secs(10),
            )
            .await
            .unwrap_or_default();
        if super::display::png_size(&header) == Some(size) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    false
}

/// Put both overrides back to what the AVD itself defines.
pub async fn reset(adb: &Adb, serial: &str) -> DwResult<DisplayInfo> {
    for line in ["wm size reset", "wm density reset"] {
        let out = adb
            .run(&["-s", serial, "shell", line], Duration::from_secs(20))
            .await?;
        if !out.ok() {
            return Err(DwError::new(
                ErrorCode::BackendFailure,
                "Android refused to reset the display.",
            )
            .detail(out.combined()));
        }
    }
    let info = read(adb, serial).await?;
    // The same settle as `apply`: a reset that is reported while the capture is
    // still the override's size would be a lie for about a second.
    if let (Some(width), Some(height)) = (info.width, info.height) {
        wait_for_capture(adb, serial, (width, height)).await;
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wm_size_is_read_with_and_without_an_override() {
        assert_eq!(
            parse_wm_size("Physical size: 1080x2400\n"),
            (Some((1080, 2400)), None)
        );
        assert_eq!(
            parse_wm_size("Physical size: 1080x2400\nOverride size: 720x1600\n"),
            (Some((1080, 2400)), Some((720, 1600)))
        );
        assert_eq!(parse_wm_size(""), (None, None));
    }

    #[test]
    fn wm_density_is_read_with_and_without_an_override() {
        assert_eq!(
            parse_wm_density("Physical density: 420\n"),
            (Some(420), None)
        );
        assert_eq!(
            parse_wm_density("Physical density: 420\nOverride density: 280\n"),
            (Some(420), Some(280))
        );
    }

    #[test]
    fn a_size_outside_a_real_display_is_refused() {
        assert!(validate_size(1080, 2400).is_ok());
        assert!(validate_size(320, 480).is_ok());
        assert!(validate_size(0, 0).is_err());
        assert!(validate_size(100, 2400).is_err());
        assert!(validate_size(1080, 99_999).is_err());
    }

    #[test]
    fn a_density_outside_android_range_is_refused() {
        assert!(validate_density(420).is_ok());
        assert!(validate_density(119).is_err());
        assert!(validate_density(641).is_err());
    }
}
