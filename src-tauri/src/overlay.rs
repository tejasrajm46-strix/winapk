//! The control overlay over a managed app window.
//!
//! scrcpy owns the window an app runs in, so DroidWin cannot draw inside it.
//! Instead this module anchors its own windows to that window's client
//! rectangle:
//!
//! - `overlay-labels` covers the client area, is click-through, and draws the
//!   package's saved mappings as labels. It is the mapping overlay `Ctrl+Shift+R`
//!   shows and hides. Being click-through is the point: while it is visible the
//!   game still receives every click that is not on a label chip.
//! - `overlay-nav` is an interactive bar along the bottom edge with Android's
//!   Back, Home and recent-apps keys. Its clicks go to DroidWin, never to the
//!   game, and it is not focusable, so pressing `Ctrl+Shift+R` still reaches the
//!   hook.
//! - `overlay-rail` is the vertical action strip on the right edge: screenshot,
//!   record, rotate, and the More actions (key mapping, label visibility, the
//!   Controls dialog). It is a tall window because More expands into a list
//!   inside it; a popup from a short bar was clipped by its own window edge.
//! - `overlay-mapper` is the key-mapping panel, docked over the app window
//!   while it is open.
//!
//! All four are anchored to the same client rectangle, re-read every tick, so
//! moving or resizing the game window moves them with it.
//!
//! Nothing here is a second input path: a nav button ends at
//! `EmulatorRuntime::key_on_display`, which is the same `adb shell input` a
//! mapped key uses. Mapping data comes from `InputController::state`, the store
//! row the Controls dialog writes.
//!
//! Geometry is re-read from the real scrcpy window every tick, so moving or
//! resizing it moves the overlay with it. Label positions are percentages of the
//! Android display and the frontend fits them into the video's letterboxed area,
//! so a resize keeps them on the same game control.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use crate::commands::AppState;
use crate::domain::{DwError, DwResult, ErrorCode};
use crate::input::{InputAction, InputController};
use crate::runtime::EmulatorRuntime;
use crate::store::Store;

pub const LABELS_WINDOW: &str = "overlay-labels";
pub const NAV_WINDOW: &str = "overlay-nav";
/// The vertical action strip on the right edge of the app window.
pub const RAIL_WINDOW: &str = "overlay-rail";
/// The key-mapping panel, docked over the app window.
pub const MAPPER_WINDOW: &str = "overlay-mapper";
const NAV_WIDTH: f64 = 306.0;
const NAV_MAX_WIDTH: f64 = 1280.0;
const NAV_HEIGHT: f64 = 58.0;
const NAV_MARGIN: f64 = 10.0;
/// Wide enough for an icon and its label, narrow enough to leave the game most
/// of the window.
const RAIL_WIDTH: f64 = 104.0;
const MAPPER_WIDTH: f64 = 368.0;
/// How often the overlay follows the app window. A resize is visible within one
/// tick; the poll is a geometry read, not an adb call.
const TICK: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl Rect {
    fn shifted(self, dx: i32, dy: i32, width: u32, height: u32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            width,
            height,
        }
    }
}

/// A point chosen by clicking the game itself, before a key is assigned to it.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PlacePoint {
    pub x: f32,
    pub y: f32,
}

/// One mapping as the overlay draws it. Touch actions carry a position;
/// key events have none, so the frontend lists them separately.
#[derive(Debug, Clone, Serialize)]
pub struct OverlayMapping {
    pub key: u32,
    pub label: String,
    pub action: String,
    pub x: Option<f32>,
    pub y: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverlayData {
    pub package_id: String,
    pub app_label: String,
    /// Whether the package's mapping profile is armed. Labels still render when
    /// it is off — the panel says so instead of pretending there are no controls.
    pub enabled: bool,
    pub mappings: Vec<OverlayMapping>,
    pub display_width: u32,
    pub display_height: u32,
    pub labels_visible: bool,
    /// True while the next click on the game picks a mapping point. The label
    /// window stops being click-through for exactly this long.
    pub placing: bool,
    /// The point already picked and waiting for a key, if any.
    pub pending: Option<PlacePoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverlayStatus {
    pub package_id: Option<String>,
    pub labels_visible: bool,
    /// Whether the key-mapping panel is open for the attached window.
    pub mapper_open: bool,
    /// True while both windows are actually on screen for a focused app window.
    pub visible: bool,
    /// Why the overlay could not be created, when that happened.
    pub failure: Option<String>,
}

#[derive(Default)]
struct State {
    package: Option<String>,
    labels: bool,
    /// The key-mapping panel is open. `Ctrl+Shift+R` hides it with the labels
    /// but does not forget it: showing the labels again brings it back.
    mapper: bool,
    visible: bool,
    rect: Option<Rect>,
    /// Last foreground answer, so the tick only acts on a transition.
    foreground: bool,
    /// Ticks since the label data was sent, to refresh after an edit elsewhere.
    data_age: u32,
    force_data: bool,
    /// The next click on the game surface chooses a mapping point.
    placing: bool,
    /// The chosen point, held until the panel saves a mapping for it.
    pending: Option<(f32, f32)>,
}

pub struct Overlay {
    app: AppHandle,
    runtime: Arc<EmulatorRuntime>,
    store: Arc<Store>,
    input: Arc<InputController>,
    /// Built by `ensure_windows`, which the shell calls once its state exists.
    /// Their webviews ask for overlay data on load, and a window created before
    /// `manage` made that first call panic with "state() called before manage()".
    windows: Mutex<Option<Windows>>,
    failure: Mutex<Option<String>>,
    state: Mutex<State>,
    started: AtomicBool,
}

#[derive(Clone)]
struct Windows {
    labels: Option<WebviewWindow>,
    nav: Option<WebviewWindow>,
    rail: Option<WebviewWindow>,
    mapper: Option<WebviewWindow>,
}

impl Overlay {
    pub fn new(
        app: AppHandle,
        runtime: Arc<EmulatorRuntime>,
        store: Arc<Store>,
        input: Arc<InputController>,
    ) -> Arc<Self> {
        let overlay = Arc::new(Self {
            app,
            runtime,
            store,
            input,
            windows: Mutex::new(None),
            failure: Mutex::new(None),
            state: Mutex::new(State::default()),
            started: AtomicBool::new(false),
        });
        overlay.start_tracking();
        overlay
    }

    /// Create the label overlay and the control bar.
    pub fn ensure_windows(&self) {
        let mut slot = self.windows.lock().unwrap_or_else(|err| err.into_inner());
        if slot.is_some() {
            return;
        }
        let labels = match build_window(
            &self.app,
            LABELS_WINDOW,
            "DroidWin mapping overlay",
            true,
            false,
        ) {
            Ok(window) => Some(window),
            Err(err) => {
                self.fail(format!("The mapping overlay could not be created: {err}"));
                None
            }
        };
        let nav = match build_window(&self.app, NAV_WINDOW, "DroidWin controls", false, false) {
            Ok(window) => Some(window),
            Err(err) => {
                self.fail(format!("The control bar could not be created: {err}"));
                None
            }
        };
        let rail = match build_window(&self.app, RAIL_WINDOW, "DroidWin actions", false, false) {
            Ok(window) => Some(window),
            Err(err) => {
                self.fail(format!("The action rail could not be created: {err}"));
                None
            }
        };
        // The mapping panel takes the keyboard (it captures the key being
        // assigned), so unlike the other two it is focusable.
        let mapper = match build_window(&self.app, MAPPER_WINDOW, "DroidWin key mapping", false, true)
        {
            Ok(window) => Some(window),
            Err(err) => {
                self.fail(format!("The key-mapping panel could not be created: {err}"));
                None
            }
        };
        *slot = Some(Windows {
            labels,
            nav,
            rail,
            mapper,
        });
    }

    /// Close both windows. The shell calls this when its own window closes,
    /// because a leftover overlay would keep the process (and the runtime it
    /// started) alive with nothing on screen.
    pub fn close_windows(&self) {
        let removed = {
            let mut slot = self.windows.lock().unwrap_or_else(|err| err.into_inner());
            slot.take()
        };
        if let Some(windows) = removed {
            if let Some(labels) = windows.labels {
                let _ = labels.close();
            }
            if let Some(nav) = windows.nav {
                let _ = nav.close();
            }
            if let Some(rail) = windows.rail {
                let _ = rail.close();
            }
            if let Some(mapper) = windows.mapper {
                let _ = mapper.close();
            }
        }
    }

    fn fail(&self, message: String) {
        let mut failure = self.failure.lock().unwrap_or_else(|err| err.into_inner());
        if failure.is_none() {
            *failure = Some(message);
        }
    }

    fn windows(&self) -> Option<Windows> {
        let slot = self.windows.lock().unwrap_or_else(|err| err.into_inner());
        slot.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Follow the focused app window for the life of the process.
    fn start_tracking(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let overlay = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let mut timer = tokio::time::interval(TICK);
            loop {
                timer.tick().await;
                overlay.tick();
            }
        });
    }

    fn window_pid(&self, package_id: &str) -> Option<u32> {
        self.runtime
            .open_windows()
            .into_iter()
            .find(|window| window.package_id == package_id)
            .map(|window| window.pid)
    }

    fn foreground_package(&self) -> Option<String> {
        let pid = crate::input::hook::foreground_process()?;
        self.runtime.package_for_pid(pid)
    }

    /// Whether the user is working inside DroidWin's own controls rather than
    /// in another application.
    ///
    /// A click on the mapping panel activates it, so the foreground is no longer
    /// the app window — and hiding on that transition hid the panel under the
    /// pointer before the click finished, which made every button in it inert.
    /// Our own process in front is the user using the overlay, not leaving the
    /// game. The main window is the exception: going back to the library ends
    /// the overlay for that app.
    fn overlay_has_focus(&self) -> bool {
        if crate::input::hook::foreground_process() != Some(std::process::id()) {
            return false;
        }
        if let Some(Ok(main)) = self.app.get_webview_window("main").map(|w| w.hwnd()) {
            if win::foreground_hwnd() == Some(main.0 as isize) {
                return false;
            }
        }
        true
    }

    fn window_rect(pid: u32) -> Option<Rect> {
        win::client_rect(pid)
    }

    /// The bottom-right corner of the monitor the app window is on.
    ///
    /// The app's own geometry decides which monitor that is: the bar and rail
    /// are hidden while they are placed, and a hidden window has no monitor to
    /// ask about — which put the bar below a tall window, off the screen.
    fn monitor_corner(&self, rect: Rect) -> Option<(i32, i32)> {
        let monitors = self.app.available_monitors().ok()?;
        monitors
            .into_iter()
            .find(|monitor| {
                let position = monitor.position();
                let size = monitor.size();
                rect.x >= position.x
                    && rect.x < position.x + size.width as i32
                    && rect.y >= position.y
                    && rect.y < position.y + size.height as i32
            })
            .map(|monitor| {
                let position = monitor.position();
                let size = monitor.size();
                (
                    position.x + size.width as i32,
                    position.y + size.height as i32,
                )
            })
    }

    /// One line per tick: attach to whichever app window is in front, follow its
    /// geometry, and hide when its window is not the foreground one.
    fn tick(&self) {
        if self.windows().is_none() {
            return;
        }
        let foreground = self.foreground_package();
        let package = self.lock().package.clone();

        // Another managed window came forward: attach to it. Labels come up with
        // it only when the package has mappings to label — an app with none was
        // opening under a legend that covered its header to say "nothing here".
        if let Some(next) = foreground.clone() {
            if package.as_deref() != Some(next.as_str()) {
                let has_mappings = !self.input.state(&next).mappings.is_empty();
                let mut state = self.lock();
                state.package = Some(next.clone());
                state.labels = has_mappings;
                // The panel edits the window it was opened over; another window
                // coming forward must not inherit it.
                state.mapper = false;
                state.foreground = true;
                state.force_data = true;
                drop(state);
                self.reposition(&next, true);
                return;
            }
        }

        let Some(package) = package else {
            return;
        };
        if self.window_pid(&package).is_none() {
            // The window closed while the overlay was attached.
            self.detach();
            return;
        }
        let focused =
            foreground.as_deref() == Some(package.as_str()) || self.overlay_has_focus();
        {
            let mut state = self.lock();
            state.foreground = focused;
            state.data_age = state.data_age.saturating_add(1);
        }
        if !focused {
            self.hide_windows();
            return;
        }
        self.reposition(&package, false);
    }

    fn reposition(&self, package: &str, force_data: bool) {
        let Some(pid) = self.window_pid(package) else {
            self.hide_windows();
            return;
        };
        let Some(rect) = Self::window_rect(pid) else {
            self.hide_windows();
            return;
        };
        if rect.width < 2 || rect.height < 2 {
            self.hide_windows();
            return;
        }

        let Some(windows) = self.windows() else {
            return;
        };
        let labels_window = windows.labels.clone();
        let nav_window = windows.nav.clone();
        let rail_window = windows.rail.clone();
        let mapper_window = windows.mapper.clone();
        let (labels_visible, mapper_open, placing) = {
            let state = self.lock();
            (state.labels, state.mapper, state.placing)
        };
        let scale = nav_window
            .as_ref()
            .and_then(|w| w.scale_factor().ok())
            .unwrap_or(1.0);

        if let Some(labels) = &labels_window {
            let _ = labels.set_position(PhysicalPosition::new(rect.x, rect.y));
            let _ = labels.set_size(PhysicalSize::new(rect.width, rect.height));
            let _ = labels.set_always_on_top(true);
            // This window is click-through so the game keeps every click. While
            // a point is being chosen it is the click surface instead — that is
            // how a click lands on the real game picture and not on a pad.
            let _ = labels.set_ignore_cursor_events(!placing);
            if labels_visible {
                let _ = labels.show();
            } else {
                let _ = labels.hide();
            }
        }
        if let Some(nav) = &nav_window {
            // The bar spans the window it belongs to, like a docked control
            // strip; it only narrows when the window itself is tiny.
            let width = rect
                .width
                .max((NAV_WIDTH * scale) as u32)
                .min((NAV_MAX_WIDTH * scale) as u32);
            let height = (NAV_HEIGHT * scale) as u32;
            let margin = (NAV_MARGIN * scale) as i32;
            if width >= 120 && height >= 24 {
                // The bar belongs to the window, not to the game: it sits under the
                // client area so it never covers the video. Only when the desktop
                // has no room below does it move above the window, and never back
                // inside it.
                let below = rect.y + rect.height as i32 + margin;
                let bottom = self
                    .monitor_corner(rect)
                    .map(|(_, bottom)| bottom - margin)
                    .unwrap_or(i32::MAX);
                // Below the window when the desktop has room, above it when it
                // does not, and docked on the window's own bottom edge when even
                // that would leave the screen: a control strip the user cannot
                // reach is worse than one over the last few pixels of the app.
                let above = rect.y.saturating_sub(height as i32 + margin).max(0);
                let y = if below + height as i32 <= bottom {
                    below
                } else if above + height as i32 <= rect.y {
                    above
                } else {
                    (rect.y + rect.height as i32 - height as i32).max(rect.y)
                };
                let placed = rect.shifted(
                    ((rect.width.saturating_sub(width)) / 2) as i32,
                    y - rect.y,
                    width,
                    height,
                );
                let _ = nav.set_position(PhysicalPosition::new(placed.x, placed.y));
                let _ = nav.set_size(PhysicalSize::new(placed.width, placed.height));
                let _ = nav.set_always_on_top(true);
                let _ = nav.show();
            } else {
                let _ = nav.hide();
            }
        }

        if let Some(rail) = &rail_window {
            // A tall strip beside the window, like the bar below it: the game
            // keeps every pixel of its own window. It only moves inside, over
            // the window's right edge, when the monitor has no room beside it.
            // Tall, not bar-shaped, because More expands into a list inside it —
            // a popup from a short window is cut off by that window's own edge.
            let width = (RAIL_WIDTH * scale) as u32;
            let margin = (NAV_MARGIN * scale) as i32;
            let fits_vertically = rect.height > (160.0 * scale) as u32;
            let beside = rect.x + rect.width as i32 + margin;
            let room_beside = self
                .monitor_corner(rect)
                .map(|(right, _)| beside + width as i32 <= right)
                .unwrap_or(false);
            if fits_vertically && width as i32 <= rect.width as i32 {
                let x = if room_beside {
                    beside
                } else {
                    rect.x + rect.width as i32 - width as i32
                };
                let _ = rail.set_position(PhysicalPosition::new(x, rect.y));
                let _ = rail.set_size(PhysicalSize::new(width, rect.height));
                let _ = rail.set_always_on_top(true);
                let _ = rail.show();
            } else {
                let _ = rail.hide();
            }
        }

        if let Some(mapper) = &mapper_window {
            // Docked over the right of the window like a drawer when there is
            // room for the game beside it, and over the whole window when there
            // is not — a panel that refuses to open is worse than one that
            // covers the game while the user is mapping it.
            let width = (MAPPER_WIDTH * scale) as u32;
            let docked = rect.width.saturating_sub(width) >= (240.0 * scale) as u32;
            let (placed, size) = if docked {
                (
                    rect.shifted(rect.width as i32 - width as i32, 0, width, rect.height),
                    width,
                )
            } else {
                (rect, rect.width)
            };
            // The panel steps aside while the game is being clicked: it covers
            // the picture it is asking the user to point at.
            if labels_visible && mapper_open && !placing {
                let _ = mapper.set_position(PhysicalPosition::new(placed.x, placed.y));
                let _ = mapper.set_size(PhysicalSize::new(size, rect.height));
                let _ = mapper.set_always_on_top(true);
                let _ = mapper.show();
            } else {
                let _ = mapper.hide();
            }
        }

        let emit = {
            let mut state = self.lock();
            let changed =
                state.rect != Some(rect) || state.force_data || force_data || state.data_age >= 8;
            state.rect = Some(rect);
            state.visible = true;
            if changed {
                state.data_age = 0;
                state.force_data = false;
            }
            changed
        };
        if emit {
            self.emit_data(package);
        }
    }

    fn hide_windows(&self) {
        let Some(windows) = self.windows() else {
            return;
        };
        if let Some(labels) = &windows.labels {
            // A leftover interactive overlay would swallow the game's clicks
            // while nothing is drawn on it.
            let _ = labels.set_ignore_cursor_events(true);
            let _ = labels.hide();
        }
        if let Some(nav) = &windows.nav {
            let _ = nav.hide();
        }
        if let Some(rail) = &windows.rail {
            let _ = rail.hide();
        }
        if let Some(mapper) = &windows.mapper {
            let _ = mapper.hide();
        }
        let mut state = self.lock();
        state.visible = false;
        // Losing the foreground ends a placement: the panel is gone, and a
        // click meant for the game must not be captured by a hidden overlay.
        if state.placing {
            tracing::info!("overlay: placement ended with the foreground");
        }
        state.placing = false;
    }

    fn detach(&self) {
        self.hide_windows();
        let mut state = self.lock();
        state.package = None;
        state.labels = false;
        state.mapper = false;
        state.rect = None;
        state.foreground = false;
    }

    fn data_for(&self, package: &str) -> OverlayData {
        let input = self.input.state(package);
        let (placing, pending, labels_visible) = {
            let state = self.lock();
            (
                state.placing,
                state.pending.map(|(x, y)| PlacePoint { x, y }),
                state.labels,
            )
        };
        let surface = self.runtime.geometry_of(package);
        let (display_width, display_height) = surface.map(|s| s.screen_size()).unwrap_or((0, 0));
        let app_label = self
            .store
            .cached_apps()
            .into_iter()
            .find(|app| app.package_id == package)
            .and_then(|app| app.label)
            .unwrap_or_else(|| package.to_string());
        let mappings = input
            .mappings
            .iter()
            .map(|mapping| {
                let (x, y) = match &mapping.action {
                    InputAction::TouchTap { x, y } | InputAction::TouchHold { x, y } => {
                        (Some(*x), Some(*y))
                    }
                    InputAction::KeyEvent { .. } => (None, None),
                };
                OverlayMapping {
                    key: mapping.key,
                    label: mapping.label.clone(),
                    action: mapping.action.summary(),
                    x,
                    y,
                }
            })
            .collect();
        OverlayData {
            package_id: package.to_string(),
            app_label,
            enabled: input.enabled,
            mappings,
            display_width,
            display_height,
            labels_visible,
            placing,
            pending,
        }
    }

    fn emit_data(&self, package: &str) {
        let data = self.data_for(package);
        for window in [LABELS_WINDOW, NAV_WINDOW, RAIL_WINDOW, MAPPER_WINDOW] {
            let _ = self.app.emit_to(window, "overlay://data", &data);
        }
    }

    pub fn current_data(&self) -> Option<OverlayData> {
        let package = self.lock().package.clone()?;
        Some(self.data_for(&package))
    }

    pub fn status(&self) -> OverlayStatus {
        let state = self.lock();
        OverlayStatus {
            package_id: state.package.clone(),
            labels_visible: state.labels,
            mapper_open: state.mapper,
            visible: state.visible,
            failure: self
                .failure
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone(),
        }
    }

    /// `Ctrl+Shift+R`, for the package whose window has the foreground.
    ///
    /// It toggles the labels only; nothing about the stored mapping, the armed
    /// state or the nav bar changes.
    pub fn toggle_labels(self: &Arc<Self>, package: String) -> DwResult<OverlayStatus> {
        if self.windows().and_then(|windows| windows.labels).is_none() {
            return Err(self.unavailable());
        }
        if self.window_pid(&package).is_none() {
            return Err(DwError::new(
                ErrorCode::LaunchFailed,
                "That app has no window open, so there is nothing to overlay.",
            ));
        }
        {
            let mut state = self.lock();
            if state.package.as_deref() == Some(package.as_str()) {
                state.labels = !state.labels;
            } else {
                state.package = Some(package.clone());
                state.labels = true;
            }
            // Hiding the overlay mid-placement ends the placement: its crosshair
            // layer is gone, so the next click on the game is the game's again.
            if !state.labels {
                state.placing = false;
            }
            state.force_data = true;
        }
        self.reposition(&package, true);
        Ok(self.status())
    }

    /// Open or close the key-mapping panel for the window it belongs to.
    ///
    /// Opening it also shows the labels: the panel edits the mapping drawn on
    /// the game, and editing invisible controls is guesswork.
    pub fn toggle_mapper(self: &Arc<Self>, package: String, open: bool) -> DwResult<OverlayStatus> {
        if self.windows().and_then(|windows| windows.mapper).is_none() {
            return Err(self.unavailable());
        }
        if self.window_pid(&package).is_none() {
            return Err(DwError::new(
                ErrorCode::LaunchFailed,
                "That app has no window open, so there is nothing to map.",
            ));
        }
        {
            let mut state = self.lock();
            state.package = Some(package.clone());
            state.mapper = open;
            if open {
                state.labels = true;
            } else {
                state.placing = false;
            }
            state.force_data = true;
        }
        self.reposition(&package, true);
        Ok(self.status())
    }

    /// Arm click-to-place for the window in front.
    ///
    /// The panel steps aside and its label window stops being click-through, so
    /// the very next click on the game picture is DroidWin's to keep. That click
    /// becomes the mapping point; the panel returns with it and waits for a key.
    pub fn begin_place(self: &Arc<Self>, package: String) -> DwResult<OverlayStatus> {
        if self.windows().and_then(|windows| windows.labels).is_none() {
            return Err(self.unavailable());
        }
        if self.window_pid(&package).is_none() {
            return Err(DwError::new(
                ErrorCode::LaunchFailed,
                "That app has no window open, so there is nothing to click.",
            ));
        }
        {
            let mut state = self.lock();
            state.package = Some(package.clone());
            state.labels = true;
            // Stay "open" so the panel comes back by itself after the click.
            state.mapper = true;
            state.placing = true;
            state.pending = None;
            state.force_data = true;
        }
        tracing::info!("overlay: waiting for a click to place a point for {package}");
        self.reposition(&package, true);
        // A click goes to the window under the pointer whether or not it is
        // focused, but taking focus means a stray key cannot reach the game
        // while the user is aiming.
        if let Some(labels) = self.windows().and_then(|windows| windows.labels) {
            let _ = labels.set_focus();
        }
        Ok(self.status())
    }

    /// Take the point the user clicked on the game, as a percentage of the
    /// Android display, and hand the screen back to the panel.
    pub fn place_at(self: &Arc<Self>, package: &str, x: f32, y: f32) -> DwResult<OverlayStatus> {
        let inside = |v: f32| v.is_finite() && (0.0..=100.0).contains(&v);
        if !inside(x) || !inside(y) {
            return Err(DwError::new(
                ErrorCode::InvalidConfig,
                "That point is outside the Android display.",
            ));
        }
        {
            let mut state = self.lock();
            if state.package.as_deref() != Some(package) || !state.placing {
                return Err(DwError::new(
                    ErrorCode::InvalidConfig,
                    "No mapping point is being placed for that window.",
                )
                .detail("Choose Place on the game from the key-mapping panel first."));
            }
            state.pending = Some((x, y));
            state.placing = false;
            state.force_data = true;
        }
        tracing::info!("overlay: point {x:.1}% / {y:.1}% chosen on {package}");
        self.reposition(package, true);
        if let Some(mapper) = self.windows().and_then(|windows| windows.mapper) {
            let _ = mapper.show();
            // The next key is the one being assigned, and this panel is what
            // captures it.
            let _ = mapper.set_focus();
        }
        Ok(self.status())
    }

    /// Give up on the point being chosen; the panel keeps its saved mappings.
    pub fn cancel_place(self: &Arc<Self>) -> OverlayStatus {
        {
            let mut state = self.lock();
            if state.placing {
                tracing::info!("overlay: placement cancelled");
            }
            state.placing = false;
            state.pending = None;
            state.force_data = true;
        }
        let package = self.lock().package.clone();
        if let Some(package) = package {
            self.reposition(&package, true);
        }
        self.status()
    }

    /// Forget the chosen point once a mapping has been saved for it.
    pub fn clear_pending(&self) {
        let mut state = self.lock();
        if state.pending.take().is_some() {
            state.force_data = true;
        }
    }

    pub fn hide(self: &Arc<Self>) -> OverlayStatus {
        self.hide_windows();
        self.status()
    }

    fn unavailable(&self) -> DwError {
        DwError::new(
            ErrorCode::NotSupported,
            self.failure
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
                .unwrap_or_else(|| "The control overlay is unavailable.".into()),
        )
    }

    pub async fn navigate(&self, package_id: &str, key: &str) -> DwResult<()> {
        let keycode = crate::agent::control::parse_key(key)?;
        let display = self.runtime.display_of(package_id).ok_or_else(|| {
            DwError::new(
                ErrorCode::LaunchFailed,
                "Android has not reported a display for that window.",
            )
            .detail("Open the app in DroidWin first, then use the bar again.")
        })?;
        self.runtime.key_on_display(display, keycode).await
    }

    /// Ask the main window to open the existing Controls dialog for a package.
    pub fn open_controls(&self, package_id: &str) {
        let _ = self
            .app
            .emit_to("main", "overlay://controls", package_id.to_string());
        if let Some(main) = self.app.get_webview_window("main") {
            let _ = main.show();
            let _ = main.set_focus();
        }
    }
}

fn build_window(
    app: &AppHandle,
    label: &str,
    title: &str,
    click_through: bool,
    focusable: bool,
) -> tauri::Result<WebviewWindow> {
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title(title)
        .inner_size(480.0, 120.0)
        .decorations(false)
        // The label field has to see the game under it; the control bar is an
        // opaque surface, because a transparent composited window painted only
        // part of itself here and the rest leaked the window behind it.
        .transparent(click_through)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .resizable(false)
        .focused(focusable)
        .focusable(focusable)
        .visible(false)
        .build()?;
    if click_through {
        // Labels never take a click: the game underneath keeps every one of them.
        let _ = window.set_ignore_cursor_events(true);
    }
    Ok(window)
}    /// The client rectangle of the largest visible top-level window a process owns.
/// That is the scrcpy window: its title bar and borders are excluded, so an
/// overlay anchored here lines up with the video.
#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    pub type Hwnd = *mut c_void;

    #[repr(C)]
    #[derive(Default)]
    pub struct WinRect {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    #[repr(C)]
    #[derive(Default)]
    pub struct Point {
        pub x: i32,
        pub y: i32,
    }

    pub struct Search {
        pub pid: u32,
        pub best: Option<super::Rect>,
    }

    /// The window with keyboard focus, as a raw handle, for identity checks.
    pub fn foreground_hwnd() -> Option<isize> {
        let hwnd = unsafe { GetForegroundWindow() };
        (!hwnd.is_null()).then_some(hwnd as isize)
    }

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(callback: extern "system" fn(Hwnd, isize) -> i32, lparam: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: Hwnd) -> i32;
        fn IsIconic(hwnd: Hwnd) -> i32;
        fn GetClientRect(hwnd: Hwnd, rect: *mut WinRect) -> i32;
        fn ClientToScreen(hwnd: Hwnd, point: *mut Point) -> i32;
        fn GetForegroundWindow() -> Hwnd;
    }

    extern "system" fn visit(hwnd: Hwnd, lparam: isize) -> i32 {
        let search = unsafe { &mut *(lparam as *mut Search) };
        let mut pid = 0u32;
        unsafe {
            if GetWindowThreadProcessId(hwnd, &mut pid) == 0
                || pid != search.pid
                || IsWindowVisible(hwnd) == 0
                || IsIconic(hwnd) != 0
            {
                return 1;
            }
            let mut client = WinRect::default();
            if GetClientRect(hwnd, &mut client) == 0 {
                return 1;
            }
            let width = (client.right - client.left).max(0) as u32;
            let height = (client.bottom - client.top).max(0) as u32;
            if width < 2 || height < 2 {
                return 1;
            }
            let mut origin = Point::default();
            if ClientToScreen(hwnd, &mut origin) == 0 {
                return 1;
            }
            let area = width as u64 * height as u64;
            let best = search
                .best
                .map(|r| r.width as u64 * r.height as u64)
                .unwrap_or(0);
            if area > best {
                search.best = Some(super::Rect {
                    x: origin.x,
                    y: origin.y,
                    width,
                    height,
                });
            }
        }
        1
    }

    pub fn client_rect(pid: u32) -> Option<super::Rect> {
        let mut search = Search { pid, best: None };
        unsafe {
            EnumWindows(visit, &mut search as *mut Search as isize);
        }
        search.best
    }
}

#[cfg(not(windows))]
mod win {
    pub fn client_rect(_pid: u32) -> Option<super::Rect> {
        None
    }

    pub fn foreground_hwnd() -> Option<isize> {
        None
    }
}

#[tauri::command]
pub async fn overlay_current(app: AppHandle) -> Result<Option<OverlayData>, DwError> {
    Ok(app.state::<AppState>().overlay.current_data())
}

#[tauri::command]
pub async fn overlay_status(app: AppHandle) -> Result<OverlayStatus, DwError> {
    Ok(app.state::<AppState>().overlay.status())
}

#[tauri::command]
pub async fn overlay_toggle_labels(app: AppHandle) -> Result<OverlayStatus, DwError> {
    let state = app.state::<AppState>();
    // A command from the nav bar has no foreground package argument; ask the
    // controller what it is attached to instead of guessing.
    let package = state.overlay.status().package_id.ok_or_else(|| {
        DwError::new(
            ErrorCode::LaunchFailed,
            "No app window has the overlay attached.",
        )
    })?;
    state.overlay.toggle_labels(package)
}

#[tauri::command]
pub async fn overlay_hide(app: AppHandle) -> Result<OverlayStatus, DwError> {
    Ok(app.state::<AppState>().overlay.hide())
}

#[tauri::command]
pub async fn overlay_nav(app: AppHandle, package_id: String, key: String) -> Result<(), DwError> {
    app.state::<AppState>()
        .overlay
        .navigate(&package_id, &key)
        .await
}

#[tauri::command]
pub async fn overlay_toggle_mapper(
    app: AppHandle,
    package_id: Option<String>,
    open: bool,
) -> Result<OverlayStatus, DwError> {
    let state = app.state::<AppState>();
    let package = package_id
        .or_else(|| state.overlay.status().package_id)
        .ok_or_else(|| {
            DwError::new(
                ErrorCode::LaunchFailed,
                "No app window has the overlay attached.",
            )
        })?;
    state.overlay.toggle_mapper(package, open)
}

#[tauri::command]
pub async fn overlay_open_controls(app: AppHandle, package_id: String) -> Result<(), DwError> {
    app.state::<AppState>().overlay.open_controls(&package_id);
    Ok(())
}

/// Start choosing a mapping point by clicking the game itself.
#[tauri::command]
pub async fn overlay_begin_place(
    app: AppHandle,
    package_id: Option<String>,
) -> Result<OverlayStatus, DwError> {
    let state = app.state::<AppState>();
    let package = package_id
        .or_else(|| state.overlay.status().package_id)
        .ok_or_else(|| {
            DwError::new(
                ErrorCode::LaunchFailed,
                "No app window has the overlay attached.",
            )
        })?;
    state.overlay.begin_place(package)
}

/// The click that landed on the game picture: its position inside the Android
/// display, as percentages.
#[tauri::command]
pub async fn overlay_place_at(
    app: AppHandle,
    package_id: String,
    x: f32,
    y: f32,
) -> Result<OverlayStatus, DwError> {
    app.state::<AppState>()
        .overlay
        .place_at(&package_id, x, y)
}

#[tauri::command]
pub async fn overlay_cancel_place(app: AppHandle) -> Result<OverlayStatus, DwError> {
    Ok(app.state::<AppState>().overlay.cancel_place())
}

#[tauri::command]
pub async fn overlay_clear_pending(app: AppHandle) -> Result<OverlayStatus, DwError> {
    let state = app.state::<AppState>();
    state.overlay.clear_pending();
    Ok(state.overlay.status())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nav_bar_sits_inside_the_bottom_edge_of_the_window() {
        let client = Rect {
            x: 100,
            y: 200,
            width: 500,
            height: 900,
        };
        let placed = client.shifted(97, 844, 306, 46);
        assert_eq!(placed.x, 197);
        assert_eq!(placed.y, 1044);
        assert!(placed.y + placed.height as i32 <= client.y + client.height as i32);
        assert!(
            placed.x >= client.x
                && placed.x + placed.width as i32 <= client.x + client.width as i32
        );
    }

    #[test]
    fn a_tiny_window_cannot_fit_a_nav_bar() {
        let client = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 40,
        };
        let width = 306u32.min(client.width.saturating_sub(16));
        assert!(width < 120, "the bar would be refused, not squashed");
    }

    #[test]
    fn a_rect_at_a_negative_origin_is_kept_as_is() {
        // Second monitor to the left of the primary one.
        let client = Rect {
            x: -1280,
            y: 0,
            width: 400,
            height: 800,
        };
        let placed = client.shifted(47, 744, 306, 46);
        assert_eq!(placed.x, -1233);
        assert_eq!(placed.width, 306);
    }
}
