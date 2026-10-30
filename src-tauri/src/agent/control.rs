//! The native Android-control actions an AI agent can invoke.
//!
//! Everything here goes through the same `EmulatorRuntime` and `Adb` the desktop
//! UI uses: no second emulator, no second adb server, no parallel window model.
//! Each action is display-scoped, because an app window is a virtual Android
//! display and an unscoped tap would land on whichever screen had focus.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{json, Value};

use super::stats;
use super::ui::{self, Selector};
use crate::apk;
use crate::domain::{DwError, DwResult, ErrorCode};
use crate::runtime::adb::Adb;
use crate::runtime::hardware;
use crate::runtime::sdk::Sdk;
use crate::runtime::EmulatorRuntime;
use crate::store::Store;

/// Cooperative cancellation shared with the caller. Long waits poll it instead
/// of ignoring a cancelled request until their timeout elapses.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn trip(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn tripped(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    fn check(&self) -> DwResult<()> {
        if self.tripped() {
            Err(DwError::new(
                ErrorCode::Cancelled,
                "The agent cancelled this request.",
            ))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub payload: Value,
    /// One line for the action log and the tool's text content.
    pub summary: String,
    pub display_id: Option<u32>,
    pub package_id: Option<String>,
}

impl Outcome {
    fn new(payload: Value, summary: impl Into<String>) -> Self {
        Self {
            payload,
            summary: summary.into(),
            display_id: None,
            package_id: None,
        }
    }

    fn at(mut self, display_id: Option<u32>, package_id: Option<String>) -> Self {
        self.display_id = display_id;
        self.package_id = package_id;
        self
    }
}

pub struct ControlService {
    runtime: Arc<EmulatorRuntime>,
    store: Arc<Store>,
    data_dir: PathBuf,
    log: Arc<super::ActionLog>,
    input: Option<Arc<crate::input::InputController>>,
    /// The app's one capture session. Shared rather than created here so a
    /// recording started from the command line is the same recording the
    /// desktop shows and stops.
    capture: Option<Arc<crate::capture::Capture>>,
    /// `uiautomator dump` writes one fixed remote file, so two concurrent dumps
    /// would read each other's output.
    hierarchy_gate: tokio::sync::Mutex<()>,
}

impl ControlService {
    pub fn new(
        runtime: Arc<EmulatorRuntime>,
        store: Arc<Store>,
        data_dir: PathBuf,
        log: Arc<super::ActionLog>,
        input: Option<Arc<crate::input::InputController>>,
    ) -> Self {
        Self {
            runtime,
            store,
            data_dir,
            log,
            input,
            capture: None,
            hierarchy_gate: tokio::sync::Mutex::new(()),
        }
    }

    /// Share the desktop app's capture session with the command line, so a
    /// recording has one owner and one state.
    pub fn with_capture(mut self, capture: Arc<crate::capture::Capture>) -> Self {
        self.capture = Some(capture);
        self
    }

    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    fn adb(&self) -> DwResult<(Adb, String)> {
        let sdk = Sdk::locate(None).ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeNotConfigured,
                "The Android SDK could not be located.",
            )
        })?;
        Ok((Adb::new(sdk.adb()), self.runtime.serial()))
    }

    pub async fn dispatch(&self, tool: &str, args: &Value, cancel: &Cancel) -> DwResult<Outcome> {
        cancel.check()?;
        match tool {
            "start_runtime" => {
                self.runtime
                    .ensure_ready(&self.store, "agent-start")
                    .await?;
                self.runtime_status().await
            }
            "get_runtime_status" => self.runtime_status().await,
            "list_devices" => self.devices().await,
            "list_installed_apps" => self.list_apps().await,
            "get_app_info" => self.app_info(args).await,
            "open_app" => self.open_app(args).await,
            "close_app_window" => self.close_app(args).await,
            "get_active_window" => self.active_window(args),
            "install_package" => self.install_package(args).await,
            "get_screen_state" => self.screen_state(args, cancel).await,
            "get_screenshot" => self.screenshot(args).await,
            "get_ui_hierarchy" => self.hierarchy(args).await,
            "find_ui_element" => self.find_element(args).await,
            "get_foreground_package" => self.foreground(args).await,
            "tap" => self.tap(args).await,
            "long_press" => self.long_press(args).await,
            "swipe" => self.swipe(args).await,
            "scroll" => self.scroll(args).await,
            "type_text" => self.type_text(args).await,
            "press_key" => self.press_key(args).await,
            "send_key_combination" => self.key_combination(args).await,
            "wait_for_element" => self.wait_for_element(args, cancel).await,
            "wait_for_app_state" => self.wait_for_app_state(args, cancel).await,
            "verify_action" => self.verify(args).await,
            "get_recent_runtime_errors" => self.recent_errors(args),
            "get_runtime_stats" => self.runtime_stats().await,
            other => Err(DwError::new(
                ErrorCode::InvalidRequest,
                format!("There is no WINAPK tool called {other}."),
            )),
        }
    }

    async fn runtime_status(&self) -> DwResult<Outcome> {
        let status = self.runtime.status(&self.store).await;
        let mut payload = serde_json::to_value(&status).unwrap_or(Value::Null);
        if let Some(object) = payload.as_object_mut() {
            object.insert(
                "windows".into(),
                serde_json::to_value(self.runtime.open_windows()).unwrap_or(Value::Null),
            );
            object.insert(
                "avd".into(),
                serde_json::to_value(status.avd.clone()).unwrap_or(Value::Null),
            );
        }
        Ok(Outcome::new(
            payload,
            format!("Runtime {}", status.state.as_label().to_lowercase()),
        ))
    }

    async fn devices(&self) -> DwResult<Outcome> {
        let (adb, serial) = self.adb()?;
        let devices = adb.devices().await?;
        let listed: Vec<Value> = devices
            .iter()
            .map(|d| {
                json!({
                    "serial": d.serial,
                    "state": d.state,
                    "managed": d.serial == serial,
                })
            })
            .collect();
        Ok(Outcome::new(
            json!({ "devices": listed, "managed_serial": serial }),
            format!("{} device(s), {} managed", devices.len(), serial),
        ))
    }

    async fn list_apps(&self) -> DwResult<Outcome> {
        let apps = self.runtime.list_apps(&self.store).await?;
        Ok(Outcome::new(
            json!({ "apps": apps }),
            format!("{} installed app(s)", apps.len()),
        ))
    }

    async fn app_info(&self, args: &Value) -> DwResult<Outcome> {
        let package_id = require_str(args, "package_id")?;
        let apps = self.runtime.list_apps(&self.store).await?;
        let app = apps
            .iter()
            .find(|a| a.package_id == package_id)
            .ok_or_else(|| {
                DwError::new(
                    ErrorCode::NotInstalled,
                    format!("{package_id} is not installed in the runtime."),
                )
            })?;
        let window = self
            .runtime
            .open_windows()
            .into_iter()
            .find(|w| w.package_id == package_id);
        Ok(Outcome::new(
            json!({ "app": app, "window": window }),
            format!("{package_id} is installed"),
        )
        .at(window.as_ref().and_then(|w| w.display_id), Some(package_id)))
    }

    async fn open_app(&self, args: &Value) -> DwResult<Outcome> {
        let package_id = require_str(args, "package_id")?;
        let label = self
            .store
            .cached_apps()
            .into_iter()
            .find(|c| c.package_id == package_id)
            .and_then(|c| c.label);
        let window = self
            .runtime
            .launch(&package_id, label.as_deref(), &self.store, "agent")
            .await?;
        let _ = self
            .store
            .record_launch(&package_id, crate::domain::now_ms());
        let display = self.runtime.display_of(&package_id);
        Ok(Outcome::new(
            json!({
                "window": window,
                "display_id": display,
                "foreground": self.foreground_of(display).await,
            }),
            if display.is_some() {
                format!("Opened {package_id} in its own window")
            } else {
                format!("Opened {package_id}, but Android has not reported its display yet")
            },
        )
        .at(display, Some(package_id)))
    }

    async fn close_app(&self, args: &Value) -> DwResult<Outcome> {
        let package_id = require_str(args, "package_id")?;
        if let Some(input) = &self.input {
            input.release_package(&package_id);
            input.flush().await;
        }
        self.runtime.close_window(&package_id).await?;
        Ok(Outcome::new(
            json!({ "package_id": package_id, "closed": true }),
            format!("Closed the window for {package_id}"),
        )
        .at(None, Some(package_id)))
    }

    fn active_window(&self, args: &Value) -> DwResult<Outcome> {
        let windows = self.runtime.open_windows();
        let wanted = opt_str(args, "package_id");
        let matched: Vec<_> = match &wanted {
            Some(package_id) => windows
                .iter()
                .filter(|w| &w.package_id == package_id)
                .cloned()
                .collect(),
            None => windows.clone(),
        };
        let display = matched.first().and_then(|w| w.display_id);
        Ok(Outcome::new(
            json!({ "windows": matched, "count": matched.len() }),
            format!("{} window(s) open", matched.len()),
        )
        .at(display, wanted))
    }

    async fn install_package(&self, args: &Value) -> DwResult<Outcome> {
        let path = PathBuf::from(require_str(args, "path")?);
        apk::validate(&path)?;
        let aapt2 = Sdk::locate(None).and_then(|sdk| sdk.aapt2());
        let inspect_path = path.clone();
        let inspection =
            tokio::task::spawn_blocking(move || apk::inspect(&inspect_path, aapt2.as_deref()))
                .await
                .map_err(|e| {
                    DwError::new(
                        ErrorCode::BackendFailure,
                        "The package inspector stopped unexpectedly.",
                    )
                    .detail(e.to_string())
                })??;
        // Old packages are refused unless the caller asked for the override by
        // name: the agent must have been told why, not guess it.
        let allow_old = opt_bool(args, "allow_old_target").unwrap_or(false);
        let outcome = self
            .runtime
            .install(&inspection, &self.store, "agent-install", allow_old)
            .await?;
        let launch = opt_bool(args, "launch").unwrap_or(false);
        let display = if launch {
            let window = self
                .runtime
                .launch(
                    &outcome.package_id,
                    inspection.label.as_deref(),
                    &self.store,
                    "agent-install",
                )
                .await?;
            let _ = window;
            self.runtime.display_of(&outcome.package_id)
        } else {
            None
        };
        let package_id = outcome.package_id.clone();
        Ok(Outcome::new(
            json!({ "install": outcome, "display_id": display }),
            if launch {
                format!("Installed and opened {package_id}")
            } else {
                format!("Installed {package_id}")
            },
        )
        .at(display, Some(package_id)))
    }

    /// Resolve which Android display an action belongs to.
    ///
    /// An explicit display wins. Otherwise the package's own window is looked up,
    /// first from the runtime's record and then from Android itself — never from
    /// the assumption that display 0 is the app.
    async fn resolve_display(
        &self,
        args: &Value,
    ) -> DwResult<(u32, Option<String>, Option<(u32, u32)>)> {
        let package = opt_str(args, "package_id");
        if let Some(display_id) = opt_u32(args, "display_id") {
            let focus = self.focus_of(Some(display_id)).await;
            let size = focus.as_ref().and_then(|f| f.width.zip(f.height));
            return Ok((display_id, focus.and_then(|f| f.package_id), size));
        }
        if let Some(package_id) = package.as_deref() {
            let display = match self.runtime.display_of(package_id) {
                Some(id) => Some(id),
                None => {
                    let (adb, serial) = self.adb()?;
                    adb.display_for_package(&serial, package_id).await
                }
            };
            let display = display.ok_or_else(|| {
                DwError::new(
                    ErrorCode::InvalidRequest,
                    format!(
                        "WINAPK does not know which display {package_id} is on. Open it with open_app first, or pass display_id."
                    ),
                )
            })?;
            let focus = self.focus_of(Some(display)).await;
            let size = focus.as_ref().and_then(|f| f.width.zip(f.height));
            return Ok((
                display,
                focus
                    .and_then(|f| f.package_id)
                    .or(Some(package_id.to_string())),
                size,
            ));
        }
        let focus = self.focus_of(None).await;
        let size = focus.as_ref().and_then(|f| f.width.zip(f.height));
        Ok((
            focus.as_ref().map(|f| f.display_id).unwrap_or(0),
            focus.and_then(|f| f.package_id),
            size,
        ))
    }

    async fn focus_of(&self, display_id: Option<u32>) -> Option<crate::runtime::adb::DisplayFocus> {
        let (adb, serial) = self.adb().ok()?;
        let displays = adb.display_focus(&serial).await.ok()?;
        match display_id {
            Some(id) => displays.into_iter().find(|d| d.display_id == id),
            None => displays.into_iter().find(|d| d.display_id == 0),
        }
    }

    async fn foreground_of(&self, display_id: Option<u32>) -> Option<String> {
        self.focus_of(display_id).await.and_then(|f| f.package_id)
    }

    async fn elements(&self, display: u32) -> DwResult<(Vec<ui::UiNode>, Option<String>)> {
        let _gate = self.hierarchy_gate.lock().await;
        let (adb, serial) = self.adb()?;
        let xml = adb.ui_hierarchy_xml(&serial).await?;
        // The dump is device-wide; an app on a virtual display is still one of
        // the windows Android reports, so filtering by the package that owns the
        // display is what makes the result belong to this action.
        let owner = self
            .focus_of(Some(display))
            .await
            .and_then(|f| f.package_id);
        // Without a known owner, even a non-empty device-wide dump cannot be
        // attributed to this display safely.
        let nodes = owner
            .as_deref()
            .map(|owner| ui::parse_hierarchy(&xml, Some(owner), 600))
            .unwrap_or_default();
        // Never fall back to the whole dump. It contains every window Android
        // knows about, and `uiautomator dump` usually captures the one window
        // that has system focus. Handing those nodes back as "this display's"
        // would let an agent tap an element that belongs to another app's
        // window — the one failure this product exists to prevent.
        let note = if nodes.is_empty() && owner.is_some() {
            Some(format!(
                "Android's hierarchy dump does not include {}, so this display exposes no elements right now. Games and canvas views never expose any; use get_screenshot and tap by coordinate.",
                owner.as_deref().unwrap_or("this display's app")
            ))
        } else if nodes.is_empty() {
            Some(format!(
                "Android did not report a focused app on display {display}, so its accessibility hierarchy cannot be attributed safely. Use get_screenshot, or focus the app and try again."
            ))
        } else {
            None
        };
        Ok((nodes, note))
    }

    /// An empty or unattributable hierarchy is not evidence that an element is
    /// absent. In particular, uiautomator can dump a different focused display.
    async fn observable_elements(&self, display: u32) -> DwResult<Vec<ui::UiNode>> {
        let (nodes, note) = self.elements(display).await?;
        require_observable_elements(nodes, note)
    }

    async fn hierarchy(&self, args: &Value) -> DwResult<Outcome> {
        let (display, package_id, _) = self.resolve_display(args).await?;
        let (nodes, note) = self.elements(display).await?;
        let visible = ui::interesting(&nodes);
        Ok(Outcome::new(
            json!({
                "display_id": display,
                "package_id": package_id,
                "node_count": nodes.len(),
                "elements": visible,
                "note": note,
            }),
            format!("{} element(s) on display {display}", visible.len()),
        )
        .at(Some(display), package_id))
    }

    async fn find_element(&self, args: &Value) -> DwResult<Outcome> {
        let selector = selector_from(args)?;
        if selector.is_empty() {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "Give the element a text, content_desc, resource_id, class_name or index.",
            ));
        }
        let (display, package_id, _) = self.resolve_display(args).await?;
        let (nodes, note) = self.elements(display).await?;
        let found = ui::find(&nodes, &selector).cloned();
        let summary = match &found {
            Some(node) => format!(
                "Found “{}” at {},{}",
                node.label().unwrap_or("element"),
                node.center().0,
                node.center().1
            ),
            None => format!("No element matched on display {display}"),
        };
        Ok(Outcome::new(
            json!({
                "display_id": display,
                "package_id": package_id,
                "element": found,
                "found": found.is_some(),
                "note": note,
            }),
            summary,
        )
        .at(Some(display), package_id))
    }

    async fn foreground(&self, args: &Value) -> DwResult<Outcome> {
        // Same targeting rules as every other display-scoped tool: a display id,
        // or the package whose window owns one. Without either, display 0 is the
        // main screen, which is what this reports on.
        let (resolved, _, _) = self.resolve_display(args).await?;
        let (adb, serial) = self.adb()?;
        let displays = adb.display_focus(&serial).await?;
        let target = resolved;
        let focus = displays.iter().find(|d| d.display_id == target).cloned();
        Ok(Outcome::new(
            json!({
                "display_id": target,
                "foreground_package": focus.as_ref().and_then(|f| f.package_id.clone()),
                "activity": focus.as_ref().and_then(|f| f.activity.clone()),
                "displays": displays.iter().map(|d| json!({
                    "display_id": d.display_id,
                    "package_id": d.package_id,
                    "activity": d.activity,
                    "width": d.width,
                    "height": d.height,
                })).collect::<Vec<_>>(),
            }),
            format!(
                "Display {target} shows {}",
                focus
                    .as_ref()
                    .and_then(|f| f.package_id.clone())
                    .unwrap_or_else(|| "nothing focused".into())
            ),
        )
        .at(Some(target), focus.and_then(|f| f.package_id)))
    }

    async fn screenshot(&self, args: &Value) -> DwResult<Outcome> {
        let (display, package_id, _) = self.resolve_display(args).await?;
        let (adb, serial) = self.adb()?;
        let png = adb.screenshot_png(&serial, display).await?;
        let (width, height) = png_size(&png).unwrap_or((0, 0));
        let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
        Ok(Outcome::new(
            json!({
                "display_id": display,
                "package_id": package_id,
                "width": width,
                "height": height,
                "bytes": png.len(),
                "mime_type": "image/png",
                "base64": encoded,
            }),
            format!("Captured display {display} ({width}×{height})"),
        )
        .at(Some(display), package_id))
    }

    async fn screen_state(&self, args: &Value, cancel: &Cancel) -> DwResult<Outcome> {
        let (display, package_id, size) = self.resolve_display(args).await?;
        let want_screenshot = opt_bool(args, "screenshot").unwrap_or(true);
        let want_elements = opt_bool(args, "elements").unwrap_or(true);
        cancel.check()?;
        let (elements, note) = if want_elements {
            self.elements(display).await?
        } else {
            (Vec::new(), None)
        };
        let visible = ui::interesting(&elements);
        // The geometry is the coordinate space input is measured in, so it is
        // reported at the top level and not only inside the screenshot block.
        let mut geometry = size;
        let shot = if want_screenshot {
            let (adb, serial) = self.adb()?;
            match adb.screenshot_png(&serial, display).await {
                Ok(png) => {
                    let (width, height) = png_size(&png).unwrap_or(size.unwrap_or((0, 0)));
                    geometry = Some((width, height));
                    Some(json!({
                        "mime_type": "image/png",
                        "bytes": png.len(),
                        "width": width,
                        "height": height,
                        "base64": base64::engine::general_purpose::STANDARD.encode(&png),
                    }))
                }
                Err(err) => Some(json!({ "error": err.summary })),
            }
        } else {
            None
        };
        let (width, height) = geometry.unwrap_or((0, 0));
        let foreground = self.foreground_of(Some(display)).await;
        Ok(Outcome::new(
            json!({
                "display_id": display,
                "package_id": package_id,
                "foreground_package": foreground,
                "width": width,
                "height": height,
                "screenshot": shot,
                "elements": visible.iter().take(40).cloned().collect::<Vec<_>>(),
                "element_count": visible.len(),
                "note": note,
            }),
            format!(
                "Display {display} ({width}×{height}): {} shows {} element(s)",
                foreground.clone().unwrap_or_else(|| "nothing".into()),
                visible.len()
            ),
        )
        .at(Some(display), package_id))
    }

    /// Coordinates for a tap: an explicit point, or a selector resolved against
    /// the hierarchy on the same display.
    async fn point_for(&self, args: &Value, display: u32) -> DwResult<((i32, i32), Option<Value>)> {
        if let (Some(x), Some(y)) = (opt_i32(args, "x"), opt_i32(args, "y")) {
            return Ok(((x, y), None));
        }
        let selector = selector_from(args)?;
        if selector.is_empty() {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "Give tap either x and y, or a selector for the element to tap.",
            ));
        }
        let nodes = self.observable_elements(display).await?;
        let node = ui::find(&nodes, &selector).ok_or_else(|| {
            DwError::new(
                ErrorCode::InvalidRequest,
                "No element on that display matched the selector.",
            )
        })?;
        let (x, y) = node.center();
        Ok((
            (x, y),
            Some(serde_json::to_value(node).unwrap_or(Value::Null)),
        ))
    }

    async fn tap(&self, args: &Value) -> DwResult<Outcome> {
        let (display, package_id, size) = self.resolve_display(args).await?;
        let ((x, y), element) = self.point_for(args, display).await?;
        check_point(display, size, x, y)?;
        let (adb, serial) = self.adb()?;
        adb.input(
            &serial,
            display,
            &["tap".to_string(), x.to_string(), y.to_string()],
        )
        .await?;
        Ok(Outcome::new(
            json!({ "display_id": display, "x": x, "y": y, "element": element }),
            format!("Tapped {x},{y} on display {display}"),
        )
        .at(Some(display), package_id))
    }

    async fn long_press(&self, args: &Value) -> DwResult<Outcome> {
        let (display, package_id, size) = self.resolve_display(args).await?;
        let ((x, y), _) = self.point_for(args, display).await?;
        check_point(display, size, x, y)?;
        let duration = opt_u32(args, "duration_ms").unwrap_or(700).clamp(400, 5000);
        let (adb, serial) = self.adb()?;
        adb.input(
            &serial,
            display,
            &[
                "swipe".to_string(),
                x.to_string(),
                y.to_string(),
                x.to_string(),
                y.to_string(),
                duration.to_string(),
            ],
        )
        .await?;
        Ok(Outcome::new(
            json!({ "display_id": display, "x": x, "y": y, "duration_ms": duration }),
            format!("Long-pressed {x},{y} for {duration}ms"),
        )
        .at(Some(display), package_id))
    }

    async fn swipe(&self, args: &Value) -> DwResult<Outcome> {
        let (display, package_id, size) = self.resolve_display(args).await?;
        let from_x = require_i32(args, "from_x")?;
        let from_y = require_i32(args, "from_y")?;
        let to_x = require_i32(args, "to_x")?;
        let to_y = require_i32(args, "to_y")?;
        check_point(display, size, from_x, from_y)?;
        check_point(display, size, to_x, to_y)?;
        let duration = opt_u32(args, "duration_ms").unwrap_or(300).clamp(50, 5000);
        let (adb, serial) = self.adb()?;
        adb.input(
            &serial,
            display,
            &[
                "swipe".to_string(),
                from_x.to_string(),
                from_y.to_string(),
                to_x.to_string(),
                to_y.to_string(),
                duration.to_string(),
            ],
        )
        .await?;
        Ok(Outcome::new(
            json!({
                "display_id": display,
                "from": [from_x, from_y],
                "to": [to_x, to_y],
                "duration_ms": duration,
            }),
            format!("Swiped {from_x},{from_y} → {to_x},{to_y}"),
        )
        .at(Some(display), package_id))
    }

    async fn scroll(&self, args: &Value) -> DwResult<Outcome> {
        let (display, package_id, size) = self.resolve_display(args).await?;
        let direction = require_str(args, "direction")?.to_ascii_lowercase();
        let distance = opt_u32(args, "distance").unwrap_or(0);
        let (width, height) = size.unwrap_or((540, 1170));
        let (cx, cy) = (width as i32 / 2, height as i32 / 2);
        let span = |axis: u32| -> i32 {
            let limit = axis as i32 * 3 / 10;
            if distance == 0 {
                limit
            } else {
                (distance as i32).min(axis as i32 * 4 / 10)
            }
        };
        let (from, to) = match direction.as_str() {
            "up" => ((cx, cy + span(height)), (cx, cy - span(height))),
            "down" => ((cx, cy - span(height)), (cx, cy + span(height))),
            "left" => ((cx + span(width), cy), (cx - span(width), cy)),
            "right" => ((cx - span(width), cy), (cx + span(width), cy)),
            other => {
                return Err(DwError::new(
                    ErrorCode::InvalidRequest,
                    format!("scroll direction must be up, down, left or right, not {other}."),
                ))
            }
        };
        let (adb, serial) = self.adb()?;
        adb.input(
            &serial,
            display,
            &[
                "swipe".to_string(),
                from.0.to_string(),
                from.1.to_string(),
                to.0.to_string(),
                to.1.to_string(),
                "350".to_string(),
            ],
        )
        .await?;
        Ok(Outcome::new(
            json!({ "display_id": display, "direction": direction, "from": [from.0, from.1], "to": [to.0, to.1] }),
            format!("Scrolled {direction} on display {display}"),
        )
        .at(Some(display), package_id))
    }

    async fn type_text(&self, args: &Value) -> DwResult<Outcome> {
        let text = require_str(args, "text")?;
        if text.is_empty() {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "type_text needs some text to type.",
            ));
        }
        // Android's `input text` turns characters into key events, which only
        // exists for its virtual keyboard's character map: ASCII. Refusing is
        // better than typing half a password or a mangled message.
        if let Some(bad) = text.chars().find(|c| !c.is_ascii() || c.is_control()) {
            return Err(DwError::new(
                ErrorCode::NotSupported,
                "Android's adb text input can only type printable ASCII characters.",
            )
            .detail(format!(
                "The text contains {:?}. It was not typed, so no partial input was left in the app.",
                bad
            )));
        }
        let (display, package_id, _) = self.resolve_display(args).await?;
        let payload = text.replace(' ', "%s");
        let (adb, serial) = self.adb()?;
        adb.input(&serial, display, &["text".to_string(), payload.clone()])
            .await?;
        Ok(Outcome::new(
            json!({ "display_id": display, "characters": text.chars().count() }),
            format!(
                "Typed {} character(s) on display {display}",
                text.chars().count()
            ),
        )
        .at(Some(display), package_id))
    }

    async fn press_key(&self, args: &Value) -> DwResult<Outcome> {
        let key = require_str(args, "key")?;
        let code = parse_key(&key)?;
        let (display, package_id, _) = self.resolve_display(args).await?;
        let (adb, serial) = self.adb()?;
        adb.input(
            &serial,
            display,
            &["keyevent".to_string(), code.to_string()],
        )
        .await?;
        Ok(Outcome::new(
            json!({ "display_id": display, "key": key, "keycode": code }),
            format!("Pressed {key} on display {display}"),
        )
        .at(Some(display), package_id))
    }

    async fn key_combination(&self, args: &Value) -> DwResult<Outcome> {
        let keys = args
            .get("keys")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if keys.len() < 2 {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "send_key_combination needs at least a modifier and a key, e.g. [\"CTRL_LEFT\", \"A\"].",
            ));
        }
        let codes: Vec<String> = keys
            .iter()
            .map(|key| parse_key(key).map(|code| code.to_string()))
            .collect::<DwResult<_>>()?;
        let (display, package_id, _) = self.resolve_display(args).await?;
        let mut command = vec!["keycombination".to_string()];
        command.extend(codes.iter().cloned());
        let (adb, serial) = self.adb()?;
        adb.input(&serial, display, &command).await.map_err(|err| {
            err.detail(
                "Android before 11 has no `input keycombination`; press the keys one at a time instead.",
            )
        })?;
        Ok(Outcome::new(
            json!({ "display_id": display, "keys": keys }),
            format!("Pressed {} on display {display}", keys.join("+")),
        )
        .at(Some(display), package_id))
    }

    async fn wait_for_element(&self, args: &Value, cancel: &Cancel) -> DwResult<Outcome> {
        let selector = selector_from(args)?;
        if selector.is_empty() {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "Give wait_for_element a selector to wait for.",
            ));
        }
        let timeout = Duration::from_millis(
            opt_u64(args, "timeout_ms")
                .unwrap_or(10_000)
                .clamp(500, 60_000),
        );
        let (display, package_id, _) = self.resolve_display(args).await?;
        let started = std::time::Instant::now();
        loop {
            cancel.check()?;
            let (nodes, note) = self.elements(display).await?;
            if let Some(node) = ui::find(&nodes, &selector) {
                return Ok(Outcome::new(
                    json!({
                        "display_id": display,
                        "found": true,
                        "waited_ms": started.elapsed().as_millis() as u64,
                        "element": node,
                    }),
                    format!(
                        "“{}” appeared after {}ms",
                        node.label().unwrap_or("element"),
                        started.elapsed().as_millis()
                    ),
                )
                .at(Some(display), package_id));
            }
            if started.elapsed() >= timeout {
                return Ok(Outcome::new(
                    json!({
                        "display_id": display,
                        "found": false,
                        "waited_ms": started.elapsed().as_millis() as u64,
                        "note": note,
                    }),
                    format!("No matching element after {}s", timeout.as_secs()),
                )
                .at(Some(display), package_id));
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    }

    async fn wait_for_app_state(&self, args: &Value, cancel: &Cancel) -> DwResult<Outcome> {
        let package_id = require_str(args, "package_id")?;
        let state = require_str(args, "state")?.to_ascii_lowercase();
        if !["running", "stopped", "foreground"].contains(&state.as_str()) {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "state must be running, stopped or foreground.",
            ));
        }
        let timeout = Duration::from_millis(
            opt_u64(args, "timeout_ms")
                .unwrap_or(10_000)
                .clamp(500, 60_000),
        );
        let started = std::time::Instant::now();
        loop {
            cancel.check()?;
            let (adb, serial) = self.adb()?;
            let running = adb.process_running(&serial, &package_id).await;
            let foreground = adb.display_focus(&serial).await.map(|displays| {
                displays
                    .iter()
                    .find(|d| d.package_id.as_deref() == Some(package_id.as_str()))
                    .map(|d| d.display_id)
            });
            let foreground = foreground.unwrap_or(None);
            let reached = match state.as_str() {
                "running" => running,
                "stopped" => !running,
                _ => foreground.is_some(),
            };
            let observed = format!("running={running}, foreground_on_display={foreground:?}");
            if reached {
                return Ok(Outcome::new(
                    json!({
                        "package_id": package_id,
                        "state": state,
                        "reached": true,
                        "waited_ms": started.elapsed().as_millis() as u64,
                        "display_id": foreground,
                    }),
                    format!("{package_id} reached {state}"),
                )
                .at(foreground, Some(package_id)));
            }
            if started.elapsed() >= timeout {
                return Ok(Outcome::new(
                    json!({
                        "package_id": package_id,
                        "state": state,
                        "reached": false,
                        "waited_ms": started.elapsed().as_millis() as u64,
                        "observed": observed,
                    }),
                    format!(
                        "{package_id} did not reach {state} within {}s",
                        timeout.as_secs()
                    ),
                )
                .at(foreground, Some(package_id)));
            }
            tokio::time::sleep(Duration::from_millis(350)).await;
        }
    }

    /// Compare the state Android reports now to a caller's expectation. This is
    /// how an agent checks that its action landed instead of assuming it did.
    async fn verify(&self, args: &Value) -> DwResult<Outcome> {
        let expect = args
            .get("expect")
            .ok_or_else(|| {
                DwError::new(
                    ErrorCode::InvalidRequest,
                    "verify_action needs an expect object.",
                )
            })?
            .clone();
        let mut checks: Vec<(String, bool, String)> = Vec::new();

        if let Some(package_id) = expect.get("foreground_package").and_then(Value::as_str) {
            let (adb, serial) = self.adb()?;
            let displays = adb.display_focus(&serial).await?;
            let found = displays
                .iter()
                .find(|d| d.package_id.as_deref() == Some(package_id));
            checks.push((
                "foreground_package".into(),
                found.is_some(),
                format!(
                    "{} is {}foreground",
                    package_id,
                    if found.is_some() { "" } else { "not " }
                ),
            ));
        }
        if let Some(package_id) = expect.get("app_running").and_then(Value::as_str) {
            let (adb, serial) = self.adb()?;
            let running = adb.process_running(&serial, package_id).await;
            checks.push((
                "app_running".into(),
                running,
                format!(
                    "{package_id} is {}",
                    if running { "running" } else { "stopped" }
                ),
            ));
        }
        if let Some(package_id) = expect.get("window_open").and_then(Value::as_str) {
            let open = self
                .runtime
                .open_windows()
                .iter()
                .any(|w| w.package_id == package_id);
            checks.push((
                "window_open".into(),
                open,
                format!(
                    "the window for {package_id} is {}",
                    if open { "open" } else { "closed" }
                ),
            ));
        }
        if let Some(selector_value) = expect.get("element_present") {
            let selector: Selector =
                serde_json::from_value(selector_value.clone()).map_err(|e| {
                    DwError::new(
                        ErrorCode::InvalidRequest,
                        "element_present is not a valid selector.",
                    )
                    .detail(e.to_string())
                })?;
            let (display, _, _) = self.resolve_display(args).await?;
            let nodes = self.observable_elements(display).await?;
            let present = ui::find(&nodes, &selector).is_some();
            checks.push((
                "element_present".into(),
                present,
                format!(
                    "the element is {}present",
                    if present { "" } else { "not " }
                ),
            ));
        }
        if let Some(selector_value) = expect.get("element_absent") {
            let selector: Selector =
                serde_json::from_value(selector_value.clone()).map_err(|e| {
                    DwError::new(
                        ErrorCode::InvalidRequest,
                        "element_absent is not a valid selector.",
                    )
                    .detail(e.to_string())
                })?;
            let (display, _, _) = self.resolve_display(args).await?;
            let nodes = self.observable_elements(display).await?;
            let present = ui::find(&nodes, &selector).is_some();
            checks.push((
                "element_absent".into(),
                !present,
                format!(
                    "the element is {}present",
                    if present { "" } else { "not " }
                ),
            ));
        }

        if checks.is_empty() {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "expect needs at least one of foreground_package, app_running, window_open, element_present, element_absent.",
            ));
        }
        let verified = checks.iter().all(|(_, ok, _)| *ok);
        Ok(Outcome::new(
            json!({
                "verified": verified,
                "checks": checks
                    .iter()
                    .map(|(name, ok, detail)| json!({ "check": name, "ok": ok, "detail": detail }))
                    .collect::<Vec<_>>(),
            }),
            if verified {
                "Every expectation held".to_string()
            } else {
                "At least one expectation did not hold".to_string()
            },
        ))
    }

    // ---- observations a workflow or a trigger can check -------------------

    /// Whether the package has a process. One adb call, no screenshot.
    pub async fn app_running(&self, package_id: &str) -> DwResult<bool> {
        let (adb, serial) = self.adb()?;
        Ok(adb.process_running(&serial, package_id).await)
    }

    /// What WINAPK can see about a package right now. This is the `observation`
    /// half of a workflow step: facts, not an opinion about success.
    pub async fn observe(
        &self,
        package: Option<&str>,
        display: Option<u32>,
        tool: &str,
    ) -> DwResult<Value> {
        let windows = self.runtime.open_windows();
        let window = package.and_then(|id| windows.iter().find(|w| w.package_id == id));
        let display_id = display
            .or_else(|| package.and_then(|id| self.runtime.display_of(id)))
            .or_else(|| window.and_then(|w| w.display_id));
        let running = match package {
            Some(package_id) => self.app_running(package_id).await.ok(),
            None => None,
        };
        let foreground = match display_id {
            Some(display) => self.foreground_of(Some(display)).await,
            None => None,
        };
        Ok(json!({
            "tool": tool,
            "at_ms": crate::domain::now_ms(),
            "package_id": package,
            "display_id": display_id,
            "app_running": running,
            "window_open": window.is_some(),
            "foreground_package": foreground,
        }))
    }

    /// A fingerprint of the current picture on a display. Comparing two of these
    /// says the screen changed — it does not claim what the change was.
    pub async fn screen_hash(
        &self,
        package: Option<&str>,
        display: Option<u32>,
    ) -> DwResult<Option<String>> {
        let display = match display.or_else(|| package.and_then(|id| self.runtime.display_of(id))) {
            Some(display) => display,
            None => return Ok(None),
        };
        let (adb, serial) = self.adb()?;
        let png = adb.screenshot_png(&serial, display).await?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&png, &mut hasher);
        Ok(Some(format!("{:016x}", std::hash::Hasher::finish(&hasher))))
    }

    /// Whether an element matching the selector is on the package's display.
    pub async fn has_element(
        &self,
        package_id: &str,
        display: Option<u32>,
        selector: &Selector,
    ) -> DwResult<bool> {
        let display = match display.or_else(|| self.runtime.display_of(package_id)) {
            Some(display) => display,
            None => {
                return Err(DwError::new(
                    ErrorCode::InvalidRequest,
                    format!("WINAPK does not know which display {package_id} is on."),
                )
                .detail("Open the app first, or pass display_id."))
            }
        };
        let nodes = self.observable_elements(display).await?;
        Ok(ui::find(&nodes, selector).is_some())
    }

    fn recent_errors(&self, args: &Value) -> DwResult<Outcome> {
        let limit = opt_u32(args, "limit").unwrap_or(20).clamp(1, 100) as usize;
        let failures = self.log.failures(limit);
        Ok(Outcome::new(
            json!({ "errors": failures, "count": failures.len() }),
            format!("{} failed action(s) recorded", failures.len()),
        ))
    }

    /// Run one command from the `droidwin` command line.
    ///
    /// This is not a second action layer: every arm calls the same runtime,
    /// store and controller the MCP tools and the desktop UI use. It exists so
    /// an agent can do the things the MCP surface deliberately does not offer
    /// as tools — stopping the runtime, editing performance plans, writing a
    /// mapping — without adding a dozen more tool descriptions to every client's
    /// context.
    pub async fn dispatch_cli(&self, command: &str, args: &Value) -> DwResult<Outcome> {
        match command {
            "status" => self.runtime_status().await,
            "devices" => self.devices().await,
            "runtime.start" => {
                self.runtime.ensure_ready(&self.store, "cli-start").await?;
                self.runtime_status().await
            }
            "runtime.stop" => {
                self.runtime.shutdown().await;
                self.runtime_status().await
            }
            "runtime.restart" => {
                self.runtime.shutdown().await;
                self.runtime
                    .ensure_ready(&self.store, "cli-restart")
                    .await?;
                self.runtime_status().await
            }
            "apps.list" => self.list_apps().await,
            "apps.info" => self.app_info(args).await,
            "apps.install" => self.install_package(args).await,
            "apps.open" => self.open_app(args).await,
            "apps.close" => self.close_app(args).await,
            "apps.stop" => self.stop_app_command(args).await,
            "perf.get" => self.performance_report().await,
            "perf.preset" => self.performance_preset(args).await,
            "perf.set" => self.performance_custom(args).await,
            "mapping.get" => self.mapping_state(args),
            "mapping.save" => self.mapping_save(args),
            "mapping.remove" => self.mapping_remove(args),
            "mapping.reset" => self.mapping_reset(args),
            "mapping.enable" => self.mapping_enabled(args, true),
            "mapping.disable" => self.mapping_enabled(args, false),
            "capture.screenshot" => self.capture_screenshot(args).await,
            "capture.record.start" => self.record_start().await,
            "capture.record.stop" => self.record_stop().await,
            "capture.record.status" => self.record_status().await,
            "nav.back" => self.nav(args, "BACK").await,
            "nav.home" => self.nav(args, "HOME").await,
            "nav.recents" => self.nav(args, "APP_SWITCH").await,
            other => Err(DwError::new(
                ErrorCode::InvalidRequest,
                format!("There is no WINAPK command called {other}."),
            )
            .detail("Run `droidwin tools` for the command list.")),
        }
    }

    async fn stop_app_command(&self, args: &Value) -> DwResult<Outcome> {
        let package_id = require_str(args, "package_id")?;
        if let Some(input) = &self.input {
            input.release_package(&package_id);
            input.flush().await;
        }
        self.runtime.close_window(&package_id).await?;
        self.runtime.stop_app(&package_id).await?;
        Ok(Outcome::new(
            json!({ "package_id": package_id, "stopped": true }),
            format!("Stopped {package_id}"),
        )
        .at(None, Some(package_id)))
    }

    /// The saved plan next to what Android actually reports. The two are never
    /// merged: a plan that has not been applied says so via `restart_required`.
    async fn performance_report(&self) -> DwResult<Outcome> {
        let report = self.runtime.hardware_report(&self.store).await;
        let requested = &report.plan;
        let effective = report.effective.as_ref();
        let summary = match effective.and_then(|e| e.cores) {
            Some(cores) => format!(
                "Requested {} vCPUs, {} MB, {}; Android reports {cores} vCPUs{}",
                requested.cores,
                requested.ram_mb,
                requested.gpu.as_label(),
                if report.restart_required {
                    " (restart required)"
                } else {
                    ""
                }
            ),
            None => format!(
                "Requested {} vCPUs, {} MB, {}; Android has not reported values{}",
                requested.cores,
                requested.ram_mb,
                requested.gpu.as_label(),
                if report.restart_required {
                    " (restart required)"
                } else {
                    ""
                }
            ),
        };
        Ok(Outcome::new(
            serde_json::to_value(&report).unwrap_or(Value::Null),
            summary,
        ))
    }

    async fn performance_preset(&self, args: &Value) -> DwResult<Outcome> {
        let name = require_str(args, "preset")?;
        let preset = hardware::Preset::parse(&name).ok_or_else(|| {
            DwError::new(
                ErrorCode::InvalidConfig,
                "Choose the balanced, performance or compatibility preset.",
            )
            .detail(format!("\"{name}\" is not a preset WINAPK knows."))
        })?;
        let host = hardware::host_facts();
        let plan = hardware::preset_plan(&host, preset);
        hardware::persist(&self.store, &plan)?;
        self.performance_report().await
    }

    async fn performance_custom(&self, args: &Value) -> DwResult<Outcome> {
        let host = hardware::host_facts();
        let recommended = hardware::recommend(&host);
        let mode = opt_str(args, "mode").unwrap_or_else(|| "custom".to_string());
        let plan = match mode.as_str() {
            "auto" => hardware::recommend(&host),
            "custom" => {
                let gpu = match opt_str(args, "gpu") {
                    Some(value) => hardware::GpuMode::parse(&value).ok_or_else(|| {
                        DwError::new(ErrorCode::InvalidConfig, "Choose a supported GPU mode.")
                            .detail("Use host, software or auto.")
                    })?,
                    None => recommended.gpu,
                };
                hardware::custom_plan(
                    &host,
                    opt_u32(args, "cores").unwrap_or(recommended.cores),
                    opt_u32(args, "ram_mb").unwrap_or(recommended.ram_mb),
                    gpu,
                )?
            }
            other => {
                return Err(DwError::new(
                    ErrorCode::InvalidConfig,
                    "Choose the automatic or custom performance plan.",
                )
                .detail(format!("\"{other}\" is neither auto nor custom.")))
            }
        };
        hardware::persist(&self.store, &plan)?;
        self.performance_report().await
    }

    fn input(&self) -> DwResult<&Arc<crate::input::InputController>> {
        self.input.as_ref().ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeUnavailable,
                "Keyboard mapping is not available in this process.",
            )
        })
    }

    fn mapping_state(&self, args: &Value) -> DwResult<Outcome> {
        let package_id = require_str(args, "package_id")?;
        let state = self.input()?.state(&package_id);
        let summary = format!(
            "{} mapping(s) for {package_id}, mapping {}",
            state.mappings.len(),
            if state.enabled { "on" } else { "off" }
        );
        Ok(
            Outcome::new(serde_json::to_value(&state).unwrap_or(Value::Null), summary).at(
                state.screen.as_ref().and_then(|s| s.display_id),
                Some(package_id),
            ),
        )
    }

    fn mapping_save(&self, args: &Value) -> DwResult<Outcome> {
        let package_id = require_str(args, "package_id")?;
        let key = opt_u32(args, "key").ok_or_else(|| {
            DwError::new(
                ErrorCode::InvalidRequest,
                "A mapping needs the virtual-key code to map.",
            )
            .detail("Pass --key with a key name such as W, Space or Ctrl+Shift+K.")
        })?;
        let action: crate::input::InputAction =
            serde_json::from_value(args.get("action").cloned().ok_or_else(|| {
                DwError::new(ErrorCode::InvalidRequest, "A mapping needs an action.")
                    .detail("Use --tap x,y, --hold x,y or --keycode KEYCODE_SPACE.")
            })?)
            .map_err(|e| {
                DwError::new(
                    ErrorCode::InvalidConfig,
                    "That mapping action is not valid.",
                )
                .detail(e.to_string())
            })?;
        let state = self.input()?.save(&package_id, key, action)?;
        Ok(Outcome::new(
            serde_json::to_value(&state).unwrap_or(Value::Null),
            format!("Saved {} mapping(s) for {package_id}", state.mappings.len()),
        )
        .at(
            state.screen.as_ref().and_then(|s| s.display_id),
            Some(package_id),
        ))
    }

    fn mapping_remove(&self, args: &Value) -> DwResult<Outcome> {
        let package_id = require_str(args, "package_id")?;
        let key = opt_u32(args, "key").ok_or_else(|| {
            DwError::new(
                ErrorCode::InvalidRequest,
                "Name the key to remove with --key.",
            )
        })?;
        let state = self.input()?.remove(&package_id, key)?;
        Ok(Outcome::new(
            serde_json::to_value(&state).unwrap_or(Value::Null),
            format!("Removed one mapping; {} left", state.mappings.len()),
        )
        .at(
            state.screen.as_ref().and_then(|s| s.display_id),
            Some(package_id),
        ))
    }

    fn mapping_reset(&self, args: &Value) -> DwResult<Outcome> {
        let package_id = require_str(args, "package_id")?;
        let state = self.input()?.reset(&package_id)?;
        Ok(Outcome::new(
            serde_json::to_value(&state).unwrap_or(Value::Null),
            format!("Reset the controls for {package_id}"),
        )
        .at(None, Some(package_id)))
    }

    fn mapping_enabled(&self, args: &Value, enabled: bool) -> DwResult<Outcome> {
        let package_id = require_str(args, "package_id")?;
        let state = self.input()?.set_enabled(&package_id, enabled)?;
        Ok(Outcome::new(
            serde_json::to_value(&state).unwrap_or(Value::Null),
            format!(
                "Mapping for {package_id} is now {}",
                if enabled { "on" } else { "off" }
            ),
        )
        .at(
            state.screen.as_ref().and_then(|s| s.display_id),
            Some(package_id),
        ))
    }

    /// Where a saved capture goes: the user's own Pictures/Videos folder under
    /// WINAPK, or the data directory when Windows reports neither.
    fn capture_directory(&self, video: bool) -> PathBuf {
        let base = if video {
            dirs::video_dir()
        } else {
            dirs::picture_dir()
        };
        base.unwrap_or_else(|| self.data_dir.clone())
            .join(crate::capture::FOLDER)
    }

    async fn capture_screenshot(&self, args: &Value) -> DwResult<Outcome> {
        let directory = match opt_str(args, "out") {
            Some(path) => PathBuf::from(path),
            None => self.capture_directory(false),
        };
        let shot = crate::capture::Capture::save_screenshot(&self.runtime, &directory).await?;
        Ok(Outcome::new(
            serde_json::to_value(&shot).unwrap_or(Value::Null),
            format!("Saved {} ({}×{})", shot.path, shot.width, shot.height),
        ))
    }

    fn capture(&self) -> DwResult<&Arc<crate::capture::Capture>> {
        self.capture.as_ref().ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeUnavailable,
                "Screen recording is not available in this process.",
            )
        })
    }

    async fn record_start(&self) -> DwResult<Outcome> {
        let directory = self.capture_directory(true);
        let state = self
            .capture()?
            .start_recording(&self.runtime, &directory)
            .await?;
        Ok(Outcome::new(
            serde_json::to_value(&state).unwrap_or(Value::Null),
            "Recording Android's screen".to_string(),
        ))
    }

    async fn record_stop(&self) -> DwResult<Outcome> {
        let saved = self.capture()?.stop_recording(&self.runtime).await?;
        Ok(Outcome::new(
            serde_json::to_value(&saved).unwrap_or(Value::Null),
            format!("Saved {}", saved.path),
        ))
    }

    async fn record_status(&self) -> DwResult<Outcome> {
        let state = self.capture()?.recording_state().await;
        Ok(Outcome::new(
            serde_json::to_value(&state).unwrap_or(Value::Null),
            if state.active {
                format!("Recording for {} ms", state.elapsed_ms)
            } else {
                "Not recording".to_string()
            },
        ))
    }

    /// Android's own Back / Home / recents, on the phone's display unless a
    /// package or display is named. This is the same key path the phone's
    /// toolbar uses, not a second mechanism.
    async fn nav(&self, args: &Value, key: &str) -> DwResult<Outcome> {
        let mut call = serde_json::Map::new();
        call.insert("key".into(), json!(key));
        match (opt_str(args, "package_id"), opt_u32(args, "display_id")) {
            (Some(package_id), _) => {
                call.insert("package_id".into(), json!(package_id));
            }
            (None, Some(display_id)) => {
                call.insert("display_id".into(), json!(display_id));
            }
            (None, None) => {
                call.insert(
                    "display_id".into(),
                    json!(crate::runtime::display::DEVICE_DISPLAY),
                );
            }
        }
        self.press_key(&Value::Object(call)).await
    }

    async fn runtime_stats(&self) -> DwResult<Outcome> {
        let host = hardware::host_facts();
        let emulator_pid = self.runtime.emulator_pid().await;
        let emulator = emulator_pid.and_then(stats::process_stats);
        let app = stats::process_stats(std::process::id());
        let adb = self.adb().ok();
        let (cores, mem_total, mem_available, renderer) = match &adb {
            Some((adb, serial)) => {
                let guest_mem = adb.guest_memory(serial).await;
                (
                    adb.cpu_count(serial).await,
                    guest_mem.map(|(total, _)| total),
                    guest_mem.map(|(_, available)| available),
                    adb.gles_renderer(serial).await,
                )
            }
            None => (None, None, None, None),
        };
        Ok(Outcome::new(
            json!({
                "host": host,
                "droidwin": app,
                "emulator": {
                    "pid": emulator_pid,
                    "stats": emulator,
                },
                "android": {
                    "cores": cores,
                    "memory_total_mb": mem_total,
                    "memory_available_mb": mem_available,
                    "gpu_renderer": renderer,
                    "hardware_gpu": hardware::hardware_gpu(renderer.as_deref()),
                },
            }),
            format!(
                "{} vCPUs, {} MB free in Android",
                cores.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
                mem_available
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| "?".into())
            ),
        ))
    }
}

/// Whether a command line command can change the Android screen, the runtime
/// or saved controls. The CLI applies the same pause and read-only gate as an
/// MCP tool, so an agent cannot reach through it to do what `tools/call`
/// refuses.
pub fn cli_command_is_control(command: &str) -> bool {
    !matches!(
        command,
        "status"
            | "devices"
            | "apps.list"
            | "apps.info"
            | "perf.get"
            | "mapping.get"
            | "capture.screenshot"
            | "capture.record.status"
    )
}

/// Width and height from a PNG's IHDR chunk.
///
/// Defined with the display geometry, which is the only thing that reads capture
/// sizes; re-exported here so the control API's call sites stay readable.
pub use crate::runtime::display::png_size;

/// Android key names, plus single letters/digits. Numeric keycodes pass through.
pub fn parse_key(key: &str) -> DwResult<i32> {
    let trimmed = key.trim();
    // A single digit is the digit key itself (KEYCODE_9 is 16, not 9); a longer
    // number is a keycode, which is how a caller sends a key this table lacks.
    if trimmed.len() > 1 {
        if let Ok(code) = trimmed.parse::<i32>() {
            if code > 0 {
                return Ok(code);
            }
        }
    }
    let upper = trimmed.to_ascii_uppercase();
    let named = match upper.as_str() {
        "BACK" => 4,
        "HOME" => 3,
        "ENTER" | "RETURN" => 66,
        "TAB" => 61,
        "DEL" | "BACKSPACE" => 67,
        "FORWARD_DEL" | "DELETE" => 112,
        "ESCAPE" | "ESC" => 111,
        "SPACE" => 62,
        "MENU" => 82,
        "APP_SWITCH" | "RECENTS" => 187,
        "DPAD_UP" => 19,
        "DPAD_DOWN" => 20,
        "DPAD_LEFT" => 21,
        "DPAD_RIGHT" => 22,
        "DPAD_CENTER" => 23,
        "VOLUME_UP" => 24,
        "VOLUME_DOWN" => 25,
        "POWER" => 26,
        "PAGE_UP" => 92,
        "PAGE_DOWN" => 93,
        "MOVE_HOME" => 122,
        "MOVE_END" => 123,
        "CTRL_LEFT" | "CTRL" => 113,
        "SHIFT_LEFT" | "SHIFT" => 59,
        "ALT_LEFT" | "ALT" => 57,
        "META_LEFT" | "META" => 117,
        _ => -1,
    };
    if named > 0 {
        return Ok(named);
    }
    let mut chars = upper.chars();
    if let (Some(ch), None) = (chars.next(), chars.next()) {
        if ch.is_ascii_uppercase() {
            return Ok(29 + (ch as i32 - 'A' as i32));
        }
        if ch.is_ascii_digit() {
            return Ok(7 + (ch as i32 - '0' as i32));
        }
    }
    Err(DwError::new(
        ErrorCode::InvalidRequest,
        format!("WINAPK does not know the key {trimmed}."),
    )
    .detail("Use a name such as BACK, HOME, ENTER, TAB, DEL, FORWARD_DEL, ESCAPE, or a single letter or digit."))
}

fn require_observable_elements(
    nodes: Vec<ui::UiNode>,
    note: Option<String>,
) -> DwResult<Vec<ui::UiNode>> {
    if nodes.is_empty() {
        Err(DwError::new(
            ErrorCode::RuntimeUnavailable,
            "Android's accessibility hierarchy is unavailable for this display; the element cannot be verified.",
        )
        .detail(note.unwrap_or_else(|| "Focus the app and try again, or inspect a screenshot.".into())))
    } else {
        Ok(nodes)
    }
}

fn require_str(args: &Value, key: &str) -> DwResult<String> {
    opt_str(args, key).ok_or_else(|| {
        DwError::new(
            ErrorCode::InvalidRequest,
            format!("This action needs {key}."),
        )
    })
}

/// Refuse a point that lies outside the display Android is composing.
///
/// A tap outside the panel is dropped by the input system without an error, so
/// the agent would be told its tap worked and then see nothing change. The
/// coordinates are in the display's current orientation, which is what
/// `get_screenshot` shows — a landscape window on a rotated virtual display is
/// wider than it is tall.
fn check_point(display: u32, size: Option<(u32, u32)>, x: i32, y: i32) -> DwResult<()> {
    let Some((width, height)) = size else {
        return Ok(());
    };
    if width == 0 || height == 0 {
        return Ok(());
    }
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return Err(DwError::new(
            ErrorCode::InvalidRequest,
            format!("({x},{y}) is outside display {display}, which is {width}×{height}."),
        )
        .detail(format!(
            "Android silently drops input outside the panel. Use a point inside {width}×{height}; get_screenshot for this display shows that coordinate space."
        )));
    }
    Ok(())
}

fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn opt_u32(args: &Value, key: &str) -> Option<u32> {
    args.get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn opt_u64(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(Value::as_u64)
}

fn opt_i32(args: &Value, key: &str) -> Option<i32> {
    args.get(key)
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
}

fn opt_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_bool)
}

fn require_i32(args: &Value, key: &str) -> DwResult<i32> {
    opt_i32(args, key).ok_or_else(|| {
        DwError::new(
            ErrorCode::InvalidRequest,
            format!("This action needs a whole number for {key}."),
        )
    })
}

fn selector_from(args: &Value) -> DwResult<Selector> {
    match args.get("selector") {
        Some(value) => serde_json::from_value(value.clone()).map_err(|e| {
            DwError::new(ErrorCode::InvalidRequest, "That selector is not valid.")
                .detail(e.to_string())
        }),
        None => Ok(Selector::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_dimensions_come_from_the_header() {
        let mut png = Vec::from(*b"\x89PNG\r\n\x1a\n");
        png.extend_from_slice(&[0, 0, 0, 13]);
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&540u32.to_be_bytes());
        png.extend_from_slice(&1170u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((540, 1170)));
        assert_eq!(png_size(b"not a png at all, definitely not"), None);
    }

    #[test]
    fn a_point_outside_the_display_is_refused_instead_of_silently_dropped() {
        // 1170x540 is the landscape geometry a rotated game window reports.
        let err = check_point(7, Some((1170, 540)), 400, 950).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequest);
        assert!(err.summary.contains("outside display 7"), "{}", err.summary);
        assert!(err.summary.contains("1170×540"));
        assert!(check_point(7, Some((1170, 540)), 0, 0).is_ok());
        assert!(check_point(7, Some((1170, 540)), 1169, 539).is_ok());
        // A display whose geometry Android did not report cannot be checked, so
        // the point is passed through rather than guessed at.
        assert!(check_point(7, None, 400, 950).is_ok());
        assert!(check_point(7, Some((0, 0)), 400, 950).is_ok());
    }

    #[test]
    fn key_names_and_letters_resolve_to_android_keycodes() {
        assert_eq!(parse_key("BACK").unwrap(), 4);
        assert_eq!(parse_key("enter").unwrap(), 66);
        assert_eq!(parse_key("A").unwrap(), 29);
        assert_eq!(parse_key("z").unwrap(), 54);
        assert_eq!(parse_key("0").unwrap(), 7);
        assert_eq!(parse_key("9").unwrap(), 16);
        assert!(parse_key("KEYCODE_HOME").is_err());
        assert_eq!(parse_key("ctrl_left").unwrap(), 113);
    }

    #[test]
    fn a_numeric_keycode_is_kept_as_given() {
        assert_eq!(parse_key("187").unwrap(), 187);
        assert!(parse_key("0").is_ok());
        assert!(parse_key("-4").is_err());
    }

    #[test]
    fn an_unknown_key_names_the_input_instead_of_typing_something_else() {
        let err = parse_key("HYPER").unwrap_err();
        assert!(err.summary.contains("HYPER"));
    }

    #[test]
    fn cancellation_is_reported_before_work_starts() {
        let cancel = Cancel::new();
        cancel.trip();
        let service = test_service();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let err = runtime
            .block_on(service.dispatch("tap", &json!({ "x": 1, "y": 2 }), &cancel))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Cancelled);
    }

    #[test]
    fn an_unavailable_hierarchy_cannot_prove_an_element_is_absent() {
        let err =
            require_observable_elements(Vec::new(), Some("focus the app".into())).unwrap_err();
        assert_eq!(err.code, ErrorCode::RuntimeUnavailable);
        assert_eq!(err.detail.as_deref(), Some("focus the app"));
    }

    fn test_service() -> ControlService {
        ControlService::new(
            Arc::new(EmulatorRuntime::new(
                std::env::temp_dir(),
                Arc::new(crate::runtime::NoopEvents),
            )),
            Arc::new(Store::open_in_memory().unwrap()),
            std::env::temp_dir(),
            Arc::new(super::super::ActionLog::new()),
            None,
        )
    }

    #[test]
    fn an_unknown_tool_is_not_silently_successful() {
        let service = test_service();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let err = runtime
            .block_on(service.dispatch("teleport", &json!({}), &Cancel::new()))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequest);
    }
}
