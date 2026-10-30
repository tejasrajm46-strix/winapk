//! Per-app keyboard mapping: one input path from a physical key to an Android
//! action.
//!
//! A mapping belongs to a package, never to the whole app, and it is off until
//! the user turns it on in that package's Controls panel. While it is off nothing
//! here touches the keyboard: the Windows hook passes every key to whichever
//! window has focus, which is what scrcpy's own forwarding already handles.
//!
//! While it is on, the hook decides exactly once per key press:
//!
//! ```text
//! foreground window ──▶ owning process ──▶ package (one app window)
//!        │
//!        ├─ no mapping for this key ──▶ let scrcpy have it (no duplicate input)
//!        └─ mapping found ──▶ swallow the key, dispatch the Android action
//! ```
//!
//! Swallowing is what prevents duplicates: scrcpy never receives a key that a
//! mapping owns. Everything else — text entry, shortcuts without Ctrl/Alt/Win,
//! other apps, mapping disabled — is untouched.
//!
//! Touch targets are stored as percentages of the app's display and converted
//! with `EmulatorRuntime::geometry_of`, so a resized window or a recorded
//! orientation does not move the point.

pub mod hook;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::domain::{DwError, DwResult, ErrorCode};
use crate::runtime::EmulatorRuntime;
use crate::store::{Store, StoredMapping};

/// What a mapped key does on Android.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputAction {
    /// Send an Android key event (`KEYCODE_SPACE`, `KEYCODE_DPAD_LEFT`, …).
    KeyEvent { keycode: String },
    /// Tap a point on the app's display. `x`/`y` are percentages of its size.
    TouchTap { x: f32, y: f32 },
    /// Hold a point down while the physical key is held.
    TouchHold { x: f32, y: f32 },
}

impl InputAction {
    fn validate(&self) -> DwResult<()> {
        match self {
            InputAction::KeyEvent { keycode } => {
                let valid = !keycode.is_empty()
                    && keycode.len() <= 48
                    && keycode
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
                if valid {
                    Ok(())
                } else {
                    Err(DwError::new(
                        ErrorCode::InvalidConfig,
                        "That is not an Android keycode name.",
                    )
                    .detail("Use a name like KEYCODE_SPACE or KEYCODE_A."))
                }
            }
            InputAction::TouchTap { x, y } | InputAction::TouchHold { x, y } => {
                let inside = |v: f32| v.is_finite() && (0.0..=100.0).contains(&v);
                if inside(*x) && inside(*y) {
                    Ok(())
                } else {
                    Err(DwError::new(
                        ErrorCode::InvalidConfig,
                        "A touch target must be between 0% and 100% of the display.",
                    ))
                }
            }
        }
    }

    fn stored(&self) -> (&'static str, String) {
        match self {
            InputAction::KeyEvent { keycode } => ("key_event", keycode.clone()),
            InputAction::TouchTap { x, y } => ("touch_tap", format!("{x:.2},{y:.2}")),
            InputAction::TouchHold { x, y } => ("touch_hold", format!("{x:.2},{y:.2}")),
        }
    }

    fn from_stored(action: &str, target: &str) -> Option<Self> {
        match action {
            "key_event" => Some(InputAction::KeyEvent {
                keycode: target.to_string(),
            }),
            "touch_tap" | "touch_hold" => {
                let (x, y) = target.split_once(',')?;
                let (x, y) = (x.trim().parse().ok()?, y.trim().parse().ok()?);
                Some(if action == "touch_tap" {
                    InputAction::TouchTap { x, y }
                } else {
                    InputAction::TouchHold { x, y }
                })
            }
            _ => None,
        }
    }

    /// One line for the UI and the action log.
    pub fn summary(&self) -> String {
        match self {
            InputAction::KeyEvent { keycode } => format!("send {keycode}"),
            InputAction::TouchTap { x, y } => format!("tap {x:.0}% / {y:.0}%"),
            InputAction::TouchHold { x, y } => format!("hold {x:.0}% / {y:.0}%"),
        }
    }
}

/// A mapping as the Controls panel shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InputMapping {
    /// Windows virtual-key code — what the hook reports and the editor captures.
    pub key: u32,
    pub label: String,
    pub action: InputAction,
    pub enabled: bool,
}

/// One package's mapping state, including the coordinate space its touch targets
/// are measured in. The panel shows that space instead of assuming it.
#[derive(Debug, Clone, Serialize)]
pub struct InputState {
    pub package_id: String,
    pub enabled: bool,
    pub mappings: Vec<InputMapping>,
    pub screen: Option<SurfaceInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SurfaceInfo {
    pub display_id: Option<u32>,
    pub width: u32,
    pub height: u32,
    pub orientation: String,
}

#[derive(Debug, Clone)]
struct Profile {
    enabled: bool,
    mappings: HashMap<u32, InputAction>,
}

/// One already-resolved Android input command, waiting for the dispatcher.
enum Dispatch {
    Input { display_id: u32, args: Vec<String> },
    Barrier(tokio::sync::oneshot::Sender<()>),
}

/// Where a held touch was pressed, so the release uses the same point.
#[derive(Debug, Clone, Copy)]
struct HeldTouch {
    display_id: u32,
    x: u32,
    y: u32,
}

/// An Android action every app shares, on a fixed shortcut.
///
/// These are decided before any per-app mapping, so a game profile can never
/// swallow Android's own navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UniversalAction {
    Back,
    Home,
    Recents,
    Menu,
    Screenshot,
    Rotate,
    /// Show or hide the mapping overlay of the app in front. It is still a
    /// universal action: it is decided before any per-app profile and it is
    /// rebindable like the others.
    ShowHide,
}

impl UniversalAction {
    /// The Android key this action sends, for the four that are just keys.
    pub fn android_key(self) -> Option<&'static str> {
        match self {
            UniversalAction::Back => Some("KEYCODE_BACK"),
            UniversalAction::Home => Some("KEYCODE_HOME"),
            UniversalAction::Recents => Some("KEYCODE_APP_SWITCH"),
            UniversalAction::Menu => Some("KEYCODE_MENU"),
            UniversalAction::Screenshot | UniversalAction::Rotate | UniversalAction::ShowHide => {
                None
            }
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            UniversalAction::Back => "Back",
            UniversalAction::Home => "Home",
            UniversalAction::Recents => "Recent Apps",
            UniversalAction::Menu => "Menu",
            UniversalAction::Screenshot => "Screenshot",
            UniversalAction::Rotate => "Rotate",
            UniversalAction::ShowHide => "Show / hide mapping",
        }
    }

    /// The name the UI and the CLI use for this action.
    pub fn id(self) -> &'static str {
        match self {
            UniversalAction::Back => "back",
            UniversalAction::Home => "home",
            UniversalAction::Recents => "recents",
            UniversalAction::Menu => "menu",
            UniversalAction::Screenshot => "screenshot",
            UniversalAction::Rotate => "rotate",
            UniversalAction::ShowHide => "show_hide",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id.trim().to_ascii_lowercase().as_str() {
            "back" => Some(UniversalAction::Back),
            "home" => Some(UniversalAction::Home),
            "recents" | "recent_apps" => Some(UniversalAction::Recents),
            "menu" => Some(UniversalAction::Menu),
            "screenshot" => Some(UniversalAction::Screenshot),
            "rotate" => Some(UniversalAction::Rotate),
            "show_hide" | "showhide" | "overlay" => Some(UniversalAction::ShowHide),
            _ => None,
        }
    }
}

/// One keyboard chord: a key plus a modifier mask (1 Ctrl, 2 Alt, 4 Shift,
/// 8 Win) — the same bits the hook reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub vk: u32,
    pub modifiers: u32,
}

impl Chord {
    /// Parse `Shift+A`, `Ctrl+Shift+R`, `Alt+F4` … A universal shortcut always
    /// needs at least one modifier, so plain typing is never captured.
    pub fn parse(text: &str) -> Option<Self> {
        let mut modifiers = 0u32;
        let mut vk = None;
        for part in text.split('+').map(|p| p.trim().to_ascii_uppercase()) {
            match part.as_str() {
                "CTRL" | "CONTROL" => modifiers |= 0b0001,
                "ALT" => modifiers |= 0b0010,
                "SHIFT" => modifiers |= 0b0100,
                "WIN" | "META" => modifiers |= 0b1000,
                other => {
                    if vk.is_some() {
                        return None;
                    }
                    vk = Some(key_vk(other)?);
                }
            }
        }
        if modifiers == 0 {
            return None;
        }
        Some(Chord { vk: vk?, modifiers })
    }

    pub fn label(self) -> String {
        let mut text = String::new();
        for (bit, name) in [
            (0b0001, "Ctrl"),
            (0b0010, "Alt"),
            (0b0100, "Shift"),
            (0b1000, "Win"),
        ] {
            if self.modifiers & bit != 0 {
                text.push_str(name);
                text.push('+');
            }
        }
        text.push_str(&vk_name(self.vk));
        text
    }

    /// The value the hook and the mapping table both use for this chord.
    pub fn packed(self) -> u32 {
        self.vk | (self.modifiers << 8)
    }
}

fn key_vk(name: &str) -> Option<u32> {
    match name {
        "SPACE" => Some(0x20),
        "TAB" => Some(0x09),
        "ENTER" | "RETURN" => Some(0x0D),
        "ESCAPE" | "ESC" => Some(0x1B),
        "UP" => Some(0x26),
        "DOWN" => Some(0x28),
        "LEFT" => Some(0x25),
        "RIGHT" => Some(0x27),
        "DELETE" | "DEL" => Some(0x2E),
        "INSERT" | "INS" => Some(0x2D),
        "HOME" => Some(0x24),
        "END" => Some(0x23),
        "PAGEUP" | "PGUP" => Some(0x21),
        "PAGEDOWN" | "PGDN" => Some(0x22),
        function if function.starts_with('F') => function[1..]
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=24).contains(n))
            .map(|n| 0x70 + n - 1),
        single if single.len() == 1 => {
            let ch = single.chars().next()?;
            ch.is_ascii_alphanumeric()
                .then(|| ch.to_ascii_uppercase() as u32)
        }
        _ => None,
    }
}

fn vk_name(vk: u32) -> String {
    match vk {
        0x20 => "Space".into(),
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0x1B => "Escape".into(),
        0x26 => "Up".into(),
        0x28 => "Down".into(),
        0x25 => "Left".into(),
        0x27 => "Right".into(),
        0x2E => "Delete".into(),
        0x2D => "Insert".into(),
        0x24 => "Home".into(),
        0x23 => "End".into(),
        0x21 => "PageUp".into(),
        0x22 => "PageDown".into(),
        function @ 0x70..=0x87 => format!("F{}", function - 0x70 + 1),
        other if (0x30..=0x39).contains(&other) || (0x41..=0x5A).contains(&other) => {
            char::from_u32(other).map(String::from).unwrap_or_default()
        }
        other => format!("0x{other:02X}"),
    }
}

/// The shortcuts every app starts with.
pub fn default_universal() -> Vec<(UniversalAction, Chord)> {
    let chord = |vk| Chord {
        vk,
        modifiers: 0b0100,
    };
    vec![
        (UniversalAction::Back, chord(0x41)),
        (UniversalAction::Home, chord(0x48)),
        (UniversalAction::Recents, chord(0x52)),
        (UniversalAction::Menu, chord(0x4D)),
        (UniversalAction::Screenshot, chord(0x50)),
        (UniversalAction::Rotate, chord(0x4F)),
        // The overlay toggle is Ctrl+Shift+R, not a Shift chord: it is the one
        // universal action that changes what WINAPK draws rather than what
        // Android does, and it must not take Shift+R (Recent Apps) away.
        (
            UniversalAction::ShowHide,
            Chord {
                vk: 0x52,
                modifiers: 0b0101,
            },
        ),
    ]
}

/// The setting each shortcut's custom chord is saved under.
fn universal_setting(action: UniversalAction) -> &'static str {
    match action {
        UniversalAction::Back => "universal.back",
        UniversalAction::Home => "universal.home",
        UniversalAction::Recents => "universal.recents",
        UniversalAction::Menu => "universal.menu",
        UniversalAction::Screenshot => "universal.screenshot",
        UniversalAction::Rotate => "universal.rotate",
        UniversalAction::ShowHide => "universal.show_hide",
    }
}

/// Which action a chord is bound to, given a binding table.
pub fn action_for_chord(
    bindings: &[(UniversalAction, Chord)],
    vk: u32,
    modifiers: u32,
) -> Option<UniversalAction> {
    bindings
        .iter()
        .find(|(_, chord)| chord.vk == vk && chord.modifiers == modifiers)
        .map(|(action, _)| *action)
}

pub struct InputController {
    runtime: Arc<EmulatorRuntime>,
    store: Arc<Store>,
    profiles: Mutex<HashMap<String, Profile>>,
    /// Physical keys currently down, so auto-repeat does not fire again.
    pressed: Mutex<HashSet<(String, u32)>>,
    /// Touch points currently held down, per package and key.
    held: Mutex<HashMap<(String, u32), HeldTouch>>,
    last_foreground: Mutex<Option<String>>,
    dispatcher: mpsc::UnboundedSender<Dispatch>,
    hook_armed: AtomicBool,
    /// Where Show / hide mapping goes. Set by the overlay once its windows
    /// exist; a harness without an overlay leaves it empty and the shortcut
    /// passes through like any other unmapped key.
    overlay_toggle: OnceLock<std::sync::mpsc::Sender<String>>,
    /// Where the two universal actions that are not Android keys go (screenshot
    /// and rotate). Set once from setup; empty in a bare harness.
    universal_sink: OnceLock<std::sync::mpsc::Sender<(String, UniversalAction)>>,
    /// The last universal shortcut fired, so auto-repeat is ignored.
    universal_at: Mutex<Option<std::time::Instant>>,
    /// The chords Android's own controls are bound to; saved per action.
    universal: Mutex<Vec<(UniversalAction, Chord)>>,
}

/// Run a background task in whichever runtime this process has.
///
/// `InputController` is built during Tauri's synchronous setup, and `arm` runs
/// on the Windows hook thread; neither has a Tokio reactor in scope, so a bare
/// `tokio::spawn` panics before the first window is shown. Tauri's runtime is
/// available in both places, while the proof harnesses already run inside Tokio
/// and keep using it.
fn spawn_detached<F>(future: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::spawn(future);
    } else {
        tauri::async_runtime::spawn(future);
    }
}

impl InputController {
    pub fn new(runtime: Arc<EmulatorRuntime>, store: Arc<Store>) -> Arc<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Dispatch>();
        // Defaults, with anything the user has rebound laid over them.
        let mut universal = default_universal();
        for (action, chord) in universal.iter_mut() {
            if let Some(saved) = store.setting(universal_setting(*action)) {
                if let Some(parsed) = Chord::parse(&saved) {
                    *chord = parsed;
                }
            }
        }
        let controller = Arc::new(Self {
            runtime: Arc::clone(&runtime),
            store,
            profiles: Mutex::new(HashMap::new()),
            pressed: Mutex::new(HashSet::new()),
            held: Mutex::new(HashMap::new()),
            last_foreground: Mutex::new(None),
            dispatcher: tx,
            hook_armed: AtomicBool::new(false),
            overlay_toggle: OnceLock::new(),
            universal_sink: OnceLock::new(),
            universal_at: Mutex::new(None),
            universal: Mutex::new(universal),
        });
        // One dispatcher for the process: adb calls are serialised here instead of
        // being spawned per key press, and a release cannot overtake its press.
        spawn_detached(async move {
            while let Some(command) = rx.recv().await {
                match command {
                    Dispatch::Input { display_id, args } => {
                        let Some(adb) = runtime.adb() else {
                            tracing::warn!("mapped input was not delivered: adb is unavailable");
                            continue;
                        };
                        let serial = runtime.serial();
                        match adb.input(&serial, display_id, &args).await {
                            Ok(_) => tracing::debug!(
                                "mapped input delivered on display {display_id}: {}",
                                args.join(" ")
                            ),
                            Err(err) => {
                                tracing::warn!("mapped input was not delivered: {}", err.summary)
                            }
                        }
                    }
                    Dispatch::Barrier(done) => {
                        let _ = done.send(());
                    }
                }
            }
        });
        controller
    }

    /// Start the keyboard hook. Safe to call repeatedly; needed once.
    pub fn arm(self: &Arc<Self>) {
        if self.hook_armed.swap(true, Ordering::SeqCst) {
            return;
        }
        hook::start(Arc::clone(self));
        // Focus can change without a key press (Alt+Tab while a touch is held),
        // so held keys are also released by a bounded poll.
        let weak = Arc::downgrade(self);
        spawn_detached(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                let Some(controller) = weak.upgrade() else {
                    break;
                };
                controller.release_on_focus_change();
            }
        });
    }

    /// The universal shortcuts as they are bound right now.
    pub fn universal_bindings(&self) -> Vec<(UniversalAction, Chord)> {
        self.universal
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Rebind one Android action. A chord another action already owns, or one a
    /// saved per-app mapping already uses, is refused with an explanation rather
    /// than silently shadowed.
    pub fn set_universal_binding(
        &self,
        action: UniversalAction,
        chord: Chord,
    ) -> DwResult<Vec<(UniversalAction, Chord)>> {
        if chord.modifiers == 0 {
            return Err(DwError::new(
                ErrorCode::InvalidConfig,
                "A universal shortcut needs a modifier key.",
            )
            .detail("Use a combination like Shift+A, so plain typing is never captured."));
        }
        let bindings = self.universal_bindings();
        if let Some((other, _)) = bindings
            .iter()
            .find(|(bound, existing)| *bound != action && *existing == chord)
        {
            return Err(DwError::new(
                ErrorCode::InvalidConfig,
                format!("{} is already {}", chord.label(), other.label()),
            )
            .detail("Choose a different combination for this action."));
        }
        let taken = self.conflicting_packages(chord);
        if !taken.is_empty() {
            return Err(DwError::new(
                ErrorCode::InvalidConfig,
                format!(
                    "{} is already a mapping for {}",
                    chord.label(),
                    taken.join(", ")
                ),
            )
            .detail("Remove or change that app mapping first."));
        }
        self.store
            .set_setting(universal_setting(action), &chord.label())?;
        let mut table = self.universal.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, slot)) = table.iter_mut().find(|(bound, _)| *bound == action) {
            *slot = chord;
        }
        Ok(table.clone())
    }

    /// Packages whose saved profile already uses this exact chord.
    pub fn conflicting_packages(&self, chord: Chord) -> Vec<String> {
        let packed = chord.packed();
        let profiles = self.profiles.lock().unwrap_or_else(|e| e.into_inner());
        let mut found: Vec<String> = profiles
            .iter()
            .filter(|(_, profile)| profile.mappings.contains_key(&packed))
            .map(|(package, _)| package.clone())
            .collect();
        found.sort();
        found
    }

    pub fn state(&self, package_id: &str) -> InputState {
        InputState {
            package_id: package_id.to_string(),
            enabled: self.store.input_mapping_enabled(package_id),
            mappings: self.mappings(package_id),
            screen: self.runtime.geometry_of(package_id).map(|surface| {
                let (width, height) = surface.screen_size();
                SurfaceInfo {
                    display_id: surface.display_id,
                    width,
                    height,
                    orientation: surface.geometry.orientation.as_str().to_string(),
                }
            }),
        }
    }

    pub fn mappings(&self, package_id: &str) -> Vec<InputMapping> {
        self.store
            .input_mappings(package_id)
            .into_iter()
            .filter_map(|stored| {
                Some(InputMapping {
                    key: stored.key,
                    label: key_label(stored.key),
                    action: InputAction::from_stored(&stored.action, &stored.target)?,
                    enabled: stored.enabled,
                })
            })
            .collect()
    }

    pub fn save(&self, package_id: &str, key: u32, action: InputAction) -> DwResult<InputState> {
        if key & 0xff == 0 || key > 0x0fff || (is_modifier(key & 0xff) && key > 0xff) {
            return Err(DwError::new(
                ErrorCode::InvalidConfig,
                "That key code cannot be mapped.",
            ));
        }
        action.validate()?;
        // Replacing a binding while it is held must release the old touch before
        // the new action takes ownership of the next key-up.
        self.release_key(package_id, key);
        let (kind, target) = action.stored();
        self.store.save_input_mapping(&StoredMapping {
            package_id: package_id.to_string(),
            key,
            action: kind.to_string(),
            target,
            enabled: true,
            updated_at: crate::domain::now_ms(),
        })?;
        self.refresh(package_id)?;
        Ok(self.state(package_id))
    }

    pub fn remove(&self, package_id: &str, key: u32) -> DwResult<InputState> {
        self.store.remove_input_mapping(package_id, key)?;
        self.release_key(package_id, key);
        self.refresh(package_id)?;
        Ok(self.state(package_id))
    }

    /// Turn one mapping on or off without deleting it.
    pub fn set_mapping_enabled(
        &self,
        package_id: &str,
        key: u32,
        enabled: bool,
    ) -> DwResult<InputState> {
        let mut mappings = self.store.input_mappings(package_id);
        let Some(mapping) = mappings.iter_mut().find(|m| m.key == key) else {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "That key has no mapping for this app.",
            ));
        };
        mapping.enabled = enabled;
        self.store.save_input_mapping(mapping)?;
        if !enabled {
            self.release_key(package_id, key);
        }
        self.refresh(package_id)?;
        Ok(self.state(package_id))
    }

    /// The app-level switch. Turning it off releases anything this profile holds.
    pub fn set_enabled(self: &Arc<Self>, package_id: &str, enabled: bool) -> DwResult<InputState> {
        self.store.set_input_mapping_enabled(package_id, enabled)?;
        if enabled {
            self.arm();
        } else {
            self.release_package(package_id);
        }
        self.refresh(package_id)?;
        Ok(self.state(package_id))
    }

    /// Remove every mapping for the app and switch mapping off — the defaults.
    pub fn reset(&self, package_id: &str) -> DwResult<InputState> {
        self.release_package(package_id);
        self.store.clear_input_mappings(package_id)?;
        self.store.set_input_mapping_enabled(package_id, false)?;
        self.refresh(package_id)?;
        Ok(self.state(package_id))
    }

    /// Arm every profile the store says is on, at startup. A profile that was left
    /// enabled cannot be silently inert after a restart.
    pub fn restore(self: &Arc<Self>) {
        let packages = self.store.input_enabled_packages();
        for package in &packages {
            if let Err(err) = self.refresh(package) {
                tracing::warn!(
                    "could not load the input profile for {package}: {}",
                    err.summary
                );
            }
        }
        if !packages.is_empty() {
            self.arm();
        }
    }

    fn refresh(&self, package_id: &str) -> DwResult<()> {
        let enabled = self.store.input_mapping_enabled(package_id);
        let mut mappings = HashMap::new();
        for stored in self.store.input_mappings(package_id) {
            if !stored.enabled {
                continue;
            }
            if let Some(action) = InputAction::from_stored(&stored.action, &stored.target) {
                mappings.insert(stored.key, action);
            }
        }
        let mut profiles = self.profiles.lock().unwrap_or_else(|e| e.into_inner());
        if enabled && !mappings.is_empty() {
            profiles.insert(package_id.to_string(), Profile { enabled, mappings });
        } else {
            profiles.remove(package_id);
        }
        Ok(())
    }

    /// Decide one key event. `true` means the key was consumed here and must not
    /// reach scrcpy; `false` passes it through untouched.
    ///
    /// This is the hook's entry point: it answers "whose window is in front" and
    /// then hands the key to `on_key_for`. A live test drives `on_key_for`
    /// directly, because a harness cannot own Windows' keyboard focus.
    pub fn on_key(&self, raw_vk: u32, is_down: bool, modifiers_down: u32) -> bool {
        let Some(package) = self.foreground_package() else {
            return false;
        };
        if let Some(consumed) = self.universal_shortcut(&package, raw_vk, is_down, modifiers_down) {
            return consumed;
        }
        self.on_key_for(&package, raw_vk, is_down, modifiers_down)
    }

    /// Where screenshot and rotate go. Called once, from setup.
    pub fn set_universal_sink(&self, sender: std::sync::mpsc::Sender<(String, UniversalAction)>) {
        let _ = self.universal_sink.set(sender);
    }

    /// Android's own controls, on shortcuts that no per-app profile can take
    /// away: Shift+A Back, Shift+H Home, Shift+R Recents, Shift+M Menu, Shift+P
    /// Screenshot, Shift+O Rotate, Ctrl+Shift+R Show/hide mapping.
    ///
    /// The press is consumed, so the same chord is never also typed into the
    /// app. Releases pass, except while the chord is still held — that keeps a
    /// stuck action impossible if a key-up is missed.
    fn universal_shortcut(
        &self,
        package: &str,
        raw_vk: u32,
        is_down: bool,
        modifiers_down: u32,
    ) -> Option<bool> {
        let vk = normalize_vk(raw_vk);
        let bindings = self
            .universal
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let action = action_for_chord(&bindings, vk, modifiers_down)?;
        let mask = bindings
            .iter()
            .find(|(bound, _)| *bound == action)
            .map(|(_, chord)| chord.modifiers)
            .unwrap_or(0b0100);
        if !is_down {
            return Some(modifiers_down == mask);
        }
        // Auto-repeat must not fire the action dozens of times. A separate
        // timestamp keeps this independent of the per-app binding table.
        let now = std::time::Instant::now();
        {
            let mut last = self.universal_at.lock().unwrap_or_else(|e| e.into_inner());
            if last.is_some_and(|at| now.duration_since(at).as_millis() < 350) {
                return Some(true);
            }
            *last = Some(now);
        }
        tracing::debug!(
            "{} in {package}: universal {}",
            Chord {
                vk,
                modifiers: mask
            }
            .label(),
            action.label()
        );
        match action.android_key() {
            Some(key) => {
                let runtime = Arc::clone(&self.runtime);
                let package = package.to_string();
                spawn_detached(async move {
                    if let Ok(keycode) = crate::agent::control::parse_key(key) {
                        if let Some(display) = runtime.display_of(&package) {
                            let _ = runtime.key_on_display(display, keycode).await;
                        }
                    }
                });
            }
            None if action == UniversalAction::ShowHide => {
                // The overlay toggle is the same channel the shortcut used before
                // it became rebindable; only the table that decides the chord
                // changed.
                if let Some(sender) = self.overlay_toggle.get() {
                    let _ = sender.send(package.to_string());
                }
            }
            None => {
                if let Some(sink) = self.universal_sink.get() {
                    let _ = sink.send((package.to_string(), action));
                }
            }
        }
        Some(true)
    }

    /// Where the Show / hide mapping action is delivered. Called once, from
    /// setup; empty in a bare harness, where the shortcut then passes through.
    pub fn set_overlay_toggle(&self, sender: std::sync::mpsc::Sender<String>) {
        let _ = self.overlay_toggle.set(sender);
    }

    /// Decide one key event for one package's mapping profile.
    pub fn on_key_for(
        &self,
        package: &str,
        raw_vk: u32,
        is_down: bool,
        modifiers_down: u32,
    ) -> bool {
        // The hook reports left/right modifiers as their own codes (0xA0…) while the
        // editor captures the generic code (16). Normalise once so a key the panel
        // offers is a key the hook recognises.
        let vk = normalize_vk(raw_vk);
        // A release belongs to the binding that consumed the original press,
        // even if a modifier was released first.
        let owned = self
            .pressed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(pkg, key)| pkg == package && (key & 0xff) == vk)
            .map(|(_, key)| *key);
        let mut binding = owned.unwrap_or_else(|| {
            if is_modifier(vk) {
                vk
            } else {
                vk | (modifiers_down << 8)
            }
        });
        let action = {
            let profiles = self.profiles.lock().unwrap_or_else(|e| e.into_inner());
            match profiles.get(package) {
                Some(profile) if profile.enabled => {
                    let exact = profile.mappings.get(&binding).cloned();
                    if exact.is_none() && owned.is_none() && modifiers_down == 4 {
                        // Preserve existing ordinary Shift+key behavior unless
                        // the user configured a specific Shift combination.
                        binding = vk;
                        profile.mappings.get(&binding).cloned()
                    } else {
                        exact
                    }
                }
                _ => None,
            }
        };
        let Some(action) = action else {
            // Not mapped: the key belongs to the window, not to us.
            tracing::debug!("key {vk:#04X} in {package}: not mapped, passed to the window");
            return false;
        };
        tracing::debug!(
            "key {vk:#04X} in {package}: mapped to {} ({})",
            action.summary(),
            if is_down { "down" } else { "up" }
        );
        // A key-up belongs to the same path as its key-down. In particular,
        // shortcuts passed to scrcpy must not have their release swallowed.
        let decision = {
            let mut pressed = self.pressed.lock().unwrap_or_else(|e| e.into_inner());
            key_transition(&mut pressed, package, binding, is_down)
        };
        match decision {
            KeyDecision::Pass => false,
            KeyDecision::Consume => true,
            KeyDecision::Dispatch => {
                self.dispatch(package, binding, &action, is_down);
                true
            }
        }
    }

    fn dispatch(&self, package: &str, vk: u32, action: &InputAction, is_down: bool) {
        let Some(surface) = self.runtime.geometry_of(package) else {
            return;
        };
        let Some(display_id) = surface.display_id else {
            return;
        };
        // Percentages are resolved against the surface Android is composing right
        // now, so a resize or a recorded orientation does not move the point.
        let (width, height) = surface.screen_size();
        let point = |x: f32, y: f32| -> (u32, u32) {
            let px = (x / 100.0 * width as f32).round();
            let py = (y / 100.0 * height as f32).round();
            (
                px.clamp(0.0, width.saturating_sub(1) as f32) as u32,
                py.clamp(0.0, height.saturating_sub(1) as f32) as u32,
            )
        };
        match action {
            InputAction::KeyEvent { keycode } => {
                if is_down {
                    self.send(display_id, vec!["keyevent".into(), keycode.clone()]);
                }
            }
            InputAction::TouchTap { x, y } => {
                if is_down {
                    let (px, py) = point(*x, *y);
                    self.send(
                        display_id,
                        vec!["tap".into(), px.to_string(), py.to_string()],
                    );
                }
            }
            InputAction::TouchHold { x, y } => {
                let key = (package.to_string(), vk);
                if is_down {
                    let (px, py) = point(*x, *y);
                    {
                        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
                        if held.contains_key(&key) {
                            return;
                        }
                        held.insert(
                            key,
                            HeldTouch {
                                display_id,
                                x: px,
                                y: py,
                            },
                        );
                    }
                    self.send(
                        display_id,
                        vec![
                            "motionevent".into(),
                            "DOWN".into(),
                            px.to_string(),
                            py.to_string(),
                        ],
                    );
                } else {
                    let origin = {
                        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
                        held.remove(&key)
                    };
                    if let Some(origin) = origin {
                        self.send(
                            origin.display_id,
                            vec![
                                "motionevent".into(),
                                "UP".into(),
                                origin.x.to_string(),
                                origin.y.to_string(),
                            ],
                        );
                    }
                }
            }
        }
    }

    fn send(&self, display_id: u32, args: Vec<String>) {
        let _ = self.dispatcher.send(Dispatch::Input { display_id, args });
    }

    /// Release one held mapping, if any.
    fn release_key(&self, package: &str, vk: u32) {
        let origin = {
            let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
            held.remove(&(package.to_string(), vk))
        };
        if let Some(origin) = origin {
            let _ = self.dispatcher.send(Dispatch::Input {
                display_id: origin.display_id,
                args: vec![
                    "motionevent".into(),
                    "UP".into(),
                    origin.x.to_string(),
                    origin.y.to_string(),
                ],
            });
        }
        if let Ok(mut pressed) = self.pressed.lock() {
            pressed.remove(&(package.to_string(), vk));
        }
    }

    /// Release every touch this package holds. Called on disable, reset, window
    /// close and shutdown, so a key released outside the window cannot leave a
    /// finger pressed on the game.
    pub fn release_package(&self, package: &str) {
        let keys: Vec<u32> = {
            let held = self.held.lock().unwrap_or_else(|e| e.into_inner());
            held.keys()
                .filter(|(pkg, _)| pkg == package)
                .map(|(_, key)| *key)
                .collect()
        };
        for key in keys {
            self.release_key(package, key);
        }
        // Key-event and tap mappings are also tracked for repeat suppression,
        // even though they do not have a held touch to release.
        self.pressed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(pkg, _)| pkg != package);
    }

    pub fn release_all(&self) {
        let packages: HashSet<String> = {
            let held = self.held.lock().unwrap_or_else(|e| e.into_inner());
            let pressed = self.pressed.lock().unwrap_or_else(|e| e.into_inner());
            held.keys()
                .chain(pressed.iter())
                .map(|(pkg, _)| pkg.clone())
                .collect()
        };
        for package in packages {
            self.release_package(&package);
        }
    }

    /// Wait for queued releases before destroying the display they target.
    pub async fn flush(&self) {
        let (done, finished) = tokio::sync::oneshot::channel();
        if self.dispatcher.send(Dispatch::Barrier(done)).is_ok()
            && tokio::time::timeout(std::time::Duration::from_secs(5), finished)
                .await
                .is_err()
        {
            tracing::warn!("timed out waiting for mapped input to finish");
        }
    }

    /// The package whose window has keyboard focus, if it is one of ours.
    pub fn foreground_package(&self) -> Option<String> {
        let pid = hook::foreground_process()?;
        self.runtime.package_for_pid(pid)
    }

    /// When focus moves away from the app holding a touch, release it. A key-up
    /// delivered to another window would otherwise be lost.
    fn release_on_focus_change(&self) {
        self.reconcile_focus(self.foreground_package());
    }

    /// Apply the focus the caller observed. Split out so the release-on-blur rule
    /// is exercised for real (hold a key, take the focus away) instead of only
    /// being described in a comment.
    pub fn reconcile_focus(&self, current: Option<String>) {
        let previous = {
            let mut last = self
                .last_foreground
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let previous = last.clone();
            *last = current.clone();
            previous
        };
        if previous != current {
            if let Some(previous) = previous {
                self.release_package(&previous);
            }
        }
        // Focus may have moved before the first poll; release any tracked key
        // that no longer belongs to the focused window, even without a touch.
        let active_packages: HashSet<String> = {
            let held = self.held.lock().unwrap_or_else(|e| e.into_inner());
            let pressed = self.pressed.lock().unwrap_or_else(|e| e.into_inner());
            held.keys()
                .chain(pressed.iter())
                .map(|(pkg, _)| pkg.clone())
                .collect()
        };
        for package in active_packages {
            if current.as_deref() != Some(package.as_str()) {
                self.release_package(&package);
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum KeyDecision {
    Pass,
    Consume,
    Dispatch,
}

fn key_transition(
    pressed: &mut HashSet<(String, u32)>,
    package: &str,
    vk: u32,
    is_down: bool,
) -> KeyDecision {
    let key = (package.to_string(), vk);
    if is_down {
        // Auto-repeat is not a new press: Android already has the first one.
        if pressed.contains(&key) {
            KeyDecision::Consume
        } else {
            pressed.insert(key);
            KeyDecision::Dispatch
        }
    } else if pressed.remove(&key) {
        KeyDecision::Dispatch
    } else {
        KeyDecision::Pass
    }
}

/// Human label for a Windows virtual-key code. The editor shows the same string
/// for what it captured, so the panel and the hook agree.
/// Left/right modifier codes collapse to the generic code the editor captures.
pub fn normalize_vk(vk: u32) -> u32 {
    match vk {
        0xA0 | 0xA1 => 0x10,
        0xA2 | 0xA3 => 0x11,
        0xA4 | 0xA5 => 0x12,
        other => other,
    }
}

pub fn key_label(vk: u32) -> String {
    if vk > 0xff {
        let modifiers = vk >> 8;
        let mut prefix = String::new();
        if modifiers & 1 != 0 {
            prefix.push_str("Ctrl+");
        }
        if modifiers & 2 != 0 {
            prefix.push_str("Alt+");
        }
        if modifiers & 4 != 0 {
            prefix.push_str("Shift+");
        }
        if modifiers & 8 != 0 {
            prefix.push_str("Win+");
        }
        return format!("{}{}", prefix, key_label(vk & 0xff));
    }
    match vk {
        0x41..=0x5A => ((b'A' + (vk - 0x41) as u8) as char).to_string(),
        0x30..=0x39 => ((b'0' + (vk - 0x30) as u8) as char).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        0x20 => "Space".into(),
        0x0D => "Enter".into(),
        0x1B => "Esc".into(),
        0x09 => "Tab".into(),
        0x08 => "Backspace".into(),
        0x14 => "Caps Lock".into(),
        0x25 => "Left arrow".into(),
        0x26 => "Up arrow".into(),
        0x27 => "Right arrow".into(),
        0x28 => "Down arrow".into(),
        0x10 => "Shift".into(),
        0x11 => "Ctrl".into(),
        0x12 => "Alt".into(),
        0x5B => "Left Win".into(),
        0x5C => "Right Win".into(),
        other => format!("VK {other:#04X}"),
    }
}

pub fn is_modifier(vk: u32) -> bool {
    matches!(vk, 0x10 | 0x11 | 0x12 | 0xA0..=0xA5 | 0x5B | 0x5C)
}

/// The inverse of [`key_label`]: `W`, `Ctrl+Shift+K`, `Space`, `F5` and `0x2D`
/// become the virtual-key code the hook reports, with the modifier bits the
/// Controls editor stores (`Ctrl=1, Alt=2, Shift=4, Win=8`, kept in bits 8+).
///
/// The command line accepts this form so a mapping can be written without the
/// keyboard-capture dialog, and the result goes through the same
/// `InputController::save` validation either way.
pub fn parse_key_spec(spec: &str) -> Result<u32, String> {
    let mut modifiers = 0u32;
    let mut base: Option<u32> = None;
    for part in spec.split('+') {
        let part = part.trim();
        if part.is_empty() {
            return Err(format!("\"{spec}\" has an empty part between + signs."));
        }
        let modifier = match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => 1,
            "alt" => 2,
            "shift" => 4,
            "win" | "meta" | "super" => 8,
            _ => {
                if base.is_some() {
                    return Err(format!(
                        "\"{spec}\" names two keys; one of them must be a modifier."
                    ));
                }
                base = Some(
                    base_vk(part)
                        .ok_or_else(|| format!("WINAPK does not know the key \"{part}\"."))?,
                );
                continue;
            }
        };
        if modifiers & modifier != 0 {
            return Err(format!("\"{spec}\" repeats a modifier."));
        }
        modifiers |= modifier;
    }
    let base = base.ok_or_else(|| format!("\"{spec}\" needs a key that is not a modifier."))?;
    Ok(base | (modifiers << 8))
}

/// The non-modifier half of [`parse_key_spec`]. Named keys come first so `F5`
/// and `Left arrow` read as keys rather than as letters.
fn base_vk(name: &str) -> Option<u32> {
    let upper = name.to_ascii_uppercase();
    let lower = name.to_ascii_lowercase();
    let named = match lower.as_str() {
        "space" => 0x20,
        "enter" | "return" => 0x0D,
        "esc" | "escape" => 0x1B,
        "tab" => 0x09,
        "backspace" => 0x08,
        "delete" | "del" => 0x2E,
        "insert" | "ins" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" | "page up" => 0x21,
        "pagedown" | "page down" => 0x22,
        "capslock" | "caps lock" => 0x14,
        "left" | "left arrow" => 0x25,
        "up" | "up arrow" => 0x26,
        "right" | "right arrow" => 0x27,
        "down" | "down arrow" => 0x28,
        "ctrl" | "control" => 0x11,
        "shift" => 0x10,
        "alt" => 0x12,
        _ => 0,
    };
    if named != 0 {
        return Some(named);
    }
    if upper.len() == 1 {
        let ch = upper.chars().next()?;
        if ch.is_ascii_uppercase() {
            return Some(0x41 + (ch as u32 - 'A' as u32));
        }
        if ch.is_ascii_digit() {
            return Some(0x30 + (ch as u32 - '0' as u32));
        }
    }
    if let Some(digits) = lower.strip_prefix('f') {
        if let Ok(number) = digits.parse::<u32>() {
            if (1..=24).contains(&number) {
                return Some(0x6F + number);
            }
        }
    }
    // A raw virtual-key code, so a key this table lacks is still reachable.
    if let Some(hex) = lower.strip_prefix("0x") {
        return u32::from_str_radix(hex, 16).ok().filter(|vk| *vk > 0);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chord_round_trips_through_text() {
        for text in ["Shift+A", "Ctrl+Shift+R", "Alt+F4", "Ctrl+Alt+Delete"] {
            let chord = Chord::parse(text).unwrap_or_else(|| panic!("{text} should parse"));
            assert_eq!(chord.label(), text, "label must match the saved text");
        }
        // A shortcut with no modifier would swallow ordinary typing.
        assert_eq!(Chord::parse("A"), None);
        assert_eq!(Chord::parse("Shift+"), None);
        // One key per chord: two keys would be ambiguous.
        assert_eq!(Chord::parse("Shift+A+B"), None);
        // A repeated modifier is a typo, not a hazard.
        assert_eq!(
            Chord::parse("Shift+Shift+A"),
            Some(Chord {
                vk: 0x41,
                modifiers: 4
            })
        );
    }

    #[test]
    fn the_hook_finds_the_action_bound_to_a_chord() {
        let bindings = default_universal();
        assert_eq!(
            action_for_chord(&bindings, 0x41, 0b0100),
            Some(UniversalAction::Back)
        );
        assert_eq!(
            action_for_chord(&bindings, 0x48, 0b0100),
            Some(UniversalAction::Home)
        );
        assert_eq!(
            action_for_chord(&bindings, 0x52, 0b0100),
            Some(UniversalAction::Recents)
        );
        assert_eq!(
            action_for_chord(&bindings, 0x4D, 0b0100),
            Some(UniversalAction::Menu)
        );
        assert_eq!(
            action_for_chord(&bindings, 0x50, 0b0100),
            Some(UniversalAction::Screenshot)
        );
        assert_eq!(
            action_for_chord(&bindings, 0x4F, 0b0100),
            Some(UniversalAction::Rotate)
        );
        // Show/hide is part of the same table, so it can be rebound like the
        // rest and is decided before any per-app profile.
        assert_eq!(
            action_for_chord(&bindings, 0x52, 0b0101),
            Some(UniversalAction::ShowHide)
        );
        // A bare key, and Shift with Ctrl, belong to the app.
        assert_eq!(action_for_chord(&bindings, 0x41, 0), None);
        assert_eq!(action_for_chord(&bindings, 0x41, 0b0101), None);
        assert_eq!(action_for_chord(&bindings, 0x42, 0b0100), None);
        assert_eq!(UniversalAction::ShowHide.android_key(), None);
        assert_eq!(UniversalAction::ShowHide.id(), "show_hide");
        assert_eq!(
            UniversalAction::from_id("show_hide"),
            Some(UniversalAction::ShowHide)
        );
        assert_eq!(UniversalAction::ShowHide.label(), "Show / hide mapping");
        // The four that are Android keys name the keys the bar sends.
        assert_eq!(UniversalAction::Back.android_key(), Some("KEYCODE_BACK"));
        assert_eq!(UniversalAction::Home.android_key(), Some("KEYCODE_HOME"));
        assert_eq!(
            UniversalAction::Recents.android_key(),
            Some("KEYCODE_APP_SWITCH")
        );
        assert_eq!(UniversalAction::Menu.android_key(), Some("KEYCODE_MENU"));
        assert_eq!(UniversalAction::Screenshot.android_key(), None);
    }

    #[test]
    fn show_hide_is_rebindable_and_survives_a_restart() {
        let store = Arc::new(crate::store::Store::open_in_memory().unwrap());
        let build = || {
            let runtime = Arc::new(EmulatorRuntime::new(
                std::path::PathBuf::from("."),
                Arc::new(crate::runtime::NoopEvents),
            ));
            InputController::new(runtime, Arc::clone(&store))
        };
        let controller = build();
        let rebind = Chord::parse("Ctrl+Alt+O").unwrap();
        controller
            .set_universal_binding(UniversalAction::ShowHide, rebind)
            .expect("Ctrl+Alt+O is not taken");
        assert_eq!(
            store.setting("universal.show_hide").as_deref(),
            Some("Ctrl+Alt+O")
        );

        // A fresh controller reads the saved chord back, so a restart keeps it.
        let restarted = build();
        assert_eq!(
            action_for_chord(&restarted.universal_bindings(), rebind.vk, rebind.modifiers),
            Some(UniversalAction::ShowHide)
        );

        // Shift+R is Recent Apps, so binding Show/hide to it is refused with the
        // name of what owns it instead of silently shadowing Android's control.
        let err = controller
            .set_universal_binding(UniversalAction::ShowHide, Chord::parse("Shift+R").unwrap())
            .expect_err("Recents already owns Shift+R");
        assert!(err.summary.contains("Recent Apps"), "{}", err.summary);
    }

    #[test]
    fn the_controller_can_be_built_outside_a_tokio_runtime() {
        // Tauri's setup is synchronous: constructing the controller there must
        // not panic with "there is no reactor running". This test has no runtime
        // of its own, so a bare tokio::spawn would fail exactly as the app did.
        let store = Arc::new(crate::store::Store::open_in_memory().unwrap());
        let runtime = Arc::new(EmulatorRuntime::new(
            std::path::PathBuf::from("."),
            Arc::new(crate::runtime::NoopEvents),
        ));
        let controller = InputController::new(runtime, store);
        assert!(!controller.hook_armed.load(Ordering::SeqCst));
    }

    #[test]
    fn mapped_keys_keep_matching_down_and_up_ownership() {
        let mut pressed = HashSet::new();
        // A release with no matching press belongs to the window, not to us.
        assert_eq!(
            key_transition(&mut pressed, "app", 0x41, false),
            KeyDecision::Pass
        );
        assert_eq!(
            key_transition(&mut pressed, "app", 0x41, true),
            KeyDecision::Dispatch
        );
        // Auto-repeat: Android already has the first press.
        assert_eq!(
            key_transition(&mut pressed, "app", 0x41, true),
            KeyDecision::Consume
        );
        assert_eq!(
            key_transition(&mut pressed, "app", 0x41, false),
            KeyDecision::Dispatch
        );
        // A second release after the pair is not ours any more.
        assert_eq!(
            key_transition(&mut pressed, "app", 0x41, false),
            KeyDecision::Pass
        );
        assert!(pressed.is_empty());
    }

    #[test]
    fn releasing_a_package_clears_non_touch_key_state() {
        let store = Arc::new(crate::store::Store::open_in_memory().unwrap());
        let runtime = Arc::new(EmulatorRuntime::new(
            std::path::PathBuf::from("."),
            Arc::new(crate::runtime::NoopEvents),
        ));
        let controller = InputController::new(runtime, store);
        controller
            .pressed
            .lock()
            .unwrap()
            .insert(("app".into(), 0x41));
        controller.release_package("app");
        assert!(controller.pressed.lock().unwrap().is_empty());
    }

    #[test]
    fn labelled_keys_read_like_the_editor() {
        assert_eq!(key_label(0x4A), "J");
        assert_eq!(key_label(0x41), "A");
        assert_eq!(key_label(0x31), "1");
        assert_eq!(key_label(0x20), "Space");
        assert_eq!(key_label(0x25), "Left arrow");
        assert_eq!(key_label(0x70), "F1");
        assert_eq!(key_label(0x10), "Shift");
        assert_eq!(key_label(0x341), "Ctrl+Alt+A");
        assert_eq!(key_label(0x0F), "VK 0x0F");
    }

    #[test]
    fn written_keys_round_trip_through_the_editor_encoding() {
        assert_eq!(parse_key_spec("W"), Ok(0x57));
        assert_eq!(parse_key_spec("w"), Ok(0x57));
        assert_eq!(parse_key_spec("Space"), Ok(0x20));
        assert_eq!(parse_key_spec("F5"), Ok(0x74));
        assert_eq!(parse_key_spec("Ctrl+Shift+K"), Ok(0x4B | (5 << 8)));
        assert_eq!(parse_key_spec("shift + ctrl + k"), Ok(0x4B | (5 << 8)));
        assert_eq!(parse_key_spec("0x2D"), Ok(0x2D));
        // Anything the editor can show, this can read back.
        for key in [0x41u32, 0x20, 0x25, 0x70, 0x341] {
            assert_eq!(parse_key_spec(&key_label(key)), Ok(key), "{key:#X}");
        }
    }

    #[test]
    fn an_ambiguous_or_unknown_key_spec_is_refused() {
        for bad in ["", "Ctrl", "Ctrl+", "A+B", "Ctrl+Ctrl+A", "Hyper", "F25"] {
            assert!(parse_key_spec(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn left_and_right_modifiers_collapse_to_the_code_the_editor_captures() {
        assert_eq!(normalize_vk(0xA0), 0x10);
        assert_eq!(normalize_vk(0xA1), 0x10);
        assert_eq!(normalize_vk(0xA2), 0x11);
        assert_eq!(normalize_vk(0xA5), 0x12);
        assert_eq!(normalize_vk(0x4A), 0x4A);
    }

    #[test]
    fn modifier_keys_are_recognised_so_shortcuts_survive() {
        for vk in [0x10, 0x11, 0x12, 0xA0, 0xA2, 0xA4, 0x5B, 0x5C] {
            assert!(is_modifier(vk), "{vk:#04X}");
        }
        for vk in [0x4A, 0x20, 0x25] {
            assert!(!is_modifier(vk), "{vk:#04X}");
        }
    }

    #[test]
    fn the_touch_action_round_trips_through_one_row() {
        let action = InputAction::TouchTap { x: 12.5, y: 80.0 };
        let (kind, target) = action.stored();
        assert_eq!(kind, "touch_tap");
        assert_eq!(target, "12.50,80.00");
        assert_eq!(InputAction::from_stored(kind, &target), Some(action));

        let hold = InputAction::TouchHold { x: 0.0, y: 100.0 };
        let (kind, target) = hold.stored();
        assert_eq!(kind, "touch_hold");
        assert_eq!(InputAction::from_stored(kind, &target), Some(hold));
    }

    #[test]
    fn a_key_event_round_trips_and_keeps_its_keycode() {
        let action = InputAction::KeyEvent {
            keycode: "KEYCODE_DPAD_LEFT".into(),
        };
        let (kind, target) = action.stored();
        assert_eq!((kind, target.as_str()), ("key_event", "KEYCODE_DPAD_LEFT"));
        assert_eq!(InputAction::from_stored(kind, &target), Some(action));
    }

    #[test]
    fn an_unknown_stored_action_is_not_invented() {
        assert_eq!(InputAction::from_stored("macro", "x"), None);
        assert_eq!(InputAction::from_stored("touch_tap", "50"), None);
        assert_eq!(InputAction::from_stored("touch_tap", "a,b"), None);
    }

    #[test]
    fn touch_targets_outside_the_display_are_rejected() {
        let outside = InputAction::TouchTap { x: 120.0, y: 50.0 };
        assert_eq!(
            outside.validate().unwrap_err().code,
            ErrorCode::InvalidConfig
        );
        let negative = InputAction::TouchHold { x: -1.0, y: 50.0 };
        assert!(negative.validate().is_err());
        let edge = InputAction::TouchTap { x: 0.0, y: 100.0 };
        assert!(edge.validate().is_ok());
    }

    #[test]
    fn a_keycode_name_is_validated_before_it_reaches_adb() {
        let good = InputAction::KeyEvent {
            keycode: "KEYCODE_SPACE".into(),
        };
        assert!(good.validate().is_ok());
        for bad in ["", "keycode space", "KEYCODE;$", "a"] {
            let action = InputAction::KeyEvent {
                keycode: bad.into(),
            };
            assert!(action.validate().is_err(), "{bad}");
        }
    }
}
