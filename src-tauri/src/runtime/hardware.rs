//! Host resources and the emulator configuration derived from them.
//!
//! Only the emulator allocates vCPUs and RAM; scrcpy merely streams what Android
//! renders. So the plan below is applied to the emulator process and to nothing
//! else. A value is reported as *effective* only after Android confirms it
//! (`nproc`, `/proc/meminfo`, the GLES renderer) — never inferred from the flags
//! we passed.

use serde::{Deserialize, Serialize};

use crate::domain::{DwError, DwResult, ErrorCode};
use crate::store::Store;

pub const MIN_RAM_MB: u32 = 2048;
pub const MAX_RAM_MB: u32 = 8192;
/// The highest vCPU count this emulator actually applies.
///
/// Measured, not guessed: starting the AVD with `-cores 8` on a 24-thread host
/// produced a guest with `nproc` 6, and the emulator's own log said
/// `Emulator does not support more than 6 cores. Number of cores set to 6`.
/// Offering 8 would promise a number Android never receives.
pub const MAX_CORES: u32 = 6;
/// The values the UI offers. Even counts only: Android's scheduler gains
/// nothing from an odd, large count on a shared desktop.
pub const ALLOWED_CORES: [u32; 3] = [2, 4, 6];

const KEY_MODE: &str = "hw.mode";
const KEY_CORES: &str = "hw.cores";
const KEY_RAM: &str = "hw.ram_mb";
const KEY_GPU: &str = "hw.gpu";
const KEY_PRESET: &str = "hw.preset";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuMode {
    /// Let the emulator pick. In practice it falls back to software on this
    /// emulator build, which is why it is not the default.
    Auto,
    /// Render through the host GPU (D3D11/OpenGL). Verified headless.
    Host,
    /// Force SwiftShader when the host GPU path fails.
    Software,
}

impl GpuMode {
    pub fn as_arg(self) -> &'static str {
        match self {
            GpuMode::Auto => "auto",
            GpuMode::Host => "host",
            GpuMode::Software => "swiftshader_indirect",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(GpuMode::Auto),
            "host" => Some(GpuMode::Host),
            "software" | "swiftshader" | "swiftshader_indirect" => Some(GpuMode::Software),
            _ => None,
        }
    }

    pub fn as_label(self) -> &'static str {
        match self {
            GpuMode::Auto => "Automatic",
            GpuMode::Host => "Host GPU",
            GpuMode::Software => "Software (SwiftShader)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanMode {
    Auto,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwarePlan {
    pub mode: PlanMode,
    pub cores: u32,
    pub ram_mb: u32,
    pub gpu: GpuMode,
    /// Why these numbers were chosen, in the user's words. Never decorative.
    pub reason: String,
    /// The named preset this plan came from, if any. A custom plan has none.
    #[serde(default)]
    pub preset: Option<String>,
}

/// Named starting points. Each is a real plan with real numbers — a preset is
/// validated like a hand-written one, so a machine too small for it gets the
/// recommended values instead of an emulator that will not boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Balanced,
    Performance,
    Compatibility,
}

impl Preset {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "balanced" => Some(Preset::Balanced),
            "performance" => Some(Preset::Performance),
            "compatibility" => Some(Preset::Compatibility),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Preset::Balanced => "balanced",
            Preset::Performance => "performance",
            Preset::Compatibility => "compatibility",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Preset::Balanced => "Balanced",
            Preset::Performance => "Performance",
            Preset::Compatibility => "Compatibility",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostFacts {
    pub logical_cpus: u32,
    pub total_ram_mb: u64,
    pub available_ram_mb: u64,
    /// False when the platform did not answer; the UI must not invent a number.
    pub memory_detected: bool,
}

/// What Android actually reports once the runtime is up. Every field is optional
/// because a stopped runtime has no answer, not a zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectiveHardware {
    pub cores: Option<u32>,
    pub ram_mb: Option<u32>,
    /// The GLES renderer string from SurfaceFlinger.
    pub gpu_renderer: Option<String>,
    /// True only when that string names a real GPU backend.
    pub hardware_gpu: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareReport {
    pub host: HostFacts,
    pub plan: HardwarePlan,
    pub effective: Option<EffectiveHardware>,
    /// The emulator's own `-accel-check` answer, verbatim.
    pub acceleration: Option<String>,
    /// Whether that answer says hardware virtualisation is usable.
    pub acceleration_ok: bool,
    /// A changed plan only takes effect on the next emulator start.
    pub restart_required: bool,
    pub notes: Vec<String>,
}

#[cfg(windows)]
fn memory_mb() -> Option<(u64, u64)> {
    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
    }

    let mut status: MemoryStatusEx = unsafe { std::mem::zeroed() };
    status.length = std::mem::size_of::<MemoryStatusEx>() as u32;
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok == 0 {
        return None;
    }
    let mb = 1024 * 1024;
    Some((status.total_phys / mb, status.avail_phys / mb))
}

#[cfg(not(windows))]
fn memory_mb() -> Option<(u64, u64)> {
    None
}

pub fn host_facts() -> HostFacts {
    let logical = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    match memory_mb() {
        Some((total, available)) => HostFacts {
            logical_cpus: logical,
            total_ram_mb: total,
            available_ram_mb: available,
            memory_detected: true,
        },
        None => HostFacts {
            logical_cpus: logical,
            total_ram_mb: 0,
            available_ram_mb: 0,
            memory_detected: false,
        },
    }
}

/// Leave at least half the logical processors for Windows and the rest of the
/// desktop: the emulator is one guest among many processes.
pub fn recommended_cores(cpus: u32) -> u32 {
    match cpus {
        0..=4 => 2,
        5..=8 => 4,
        // 6 is the highest count the emulator was measured to apply: on a
        // 24-thread host, `-cores 8` booted with a guest `nproc` of 6, and cold
        // boot did not improve with more vCPUs. Recommending 8 would promise a
        // count Android never receives.
        _ => 6,
    }
}

/// A quarter of physical RAM, floored at 2 GB and capped at 6 GB: enough for
/// Android and a couple of apps without pushing Windows into paging.
pub fn recommended_ram_mb(total_mb: u64, available_mb: u64) -> u32 {
    let quarter = (total_mb / 4).min(u32::MAX as u64) as u32;
    let chosen = quarter.clamp(MIN_RAM_MB, 6144);
    // Only a genuine squeeze (under an eighth of the machine still free) lowers
    // the allocation. Comparing against "free right now" alone would shrink the
    // plan whenever the emulator is already running — the very memory use that
    // restarting with this plan would release.
    if available_mb > 0 && available_mb * 8 < total_mb {
        MIN_RAM_MB
    } else {
        chosen
    }
}

pub fn recommend(host: &HostFacts) -> HardwarePlan {
    let cores = recommended_cores(host.logical_cpus);
    let ram_mb = if host.memory_detected {
        recommended_ram_mb(host.total_ram_mb, host.available_ram_mb)
    } else {
        4096
    };
    let reason = if host.memory_detected {
        format!(
            "{} of {} logical processors and {:.1} GB for Android, leaving the rest for Windows.",
            cores,
            host.logical_cpus,
            ram_mb as f32 / 1024.0
        )
    } else {
        format!(
            "{} of {} logical processors; memory could not be read, so 4 GB is assumed.",
            cores, host.logical_cpus
        )
    };
    HardwarePlan {
        mode: PlanMode::Auto,
        cores,
        ram_mb,
        // `auto` is what silently produced SwiftShader here; the host GPU is
        // the right default and `Software` exists for the machines where it fails.
        gpu: GpuMode::Host,
        reason,
        preset: None,
    }
}

/// The vCPU count a performance preset asks for.
///
/// It is never every thread: the emulator competes with Windows, the webview and
/// scrcpy on the same machine, so half the logical processors is the ceiling and
/// 8 is the emulator's practical limit (on a 24-thread host `-cores 8` booted a
/// guest with 6). A small machine keeps its recommended count.
pub fn performance_cores(cpus: u32) -> u32 {
    let base = recommended_cores(cpus);
    let half = cpus / 2;
    ALLOWED_CORES
        .iter()
        .copied()
        .filter(|cores| *cores <= half)
        .max()
        .unwrap_or(base)
        .max(base)
}

/// Memory for the performance preset: a third of the machine, kept inside the
/// range the emulator handles and under 8 GB so Windows keeps headroom.
pub fn performance_ram_mb(total_mb: u64, available_mb: u64) -> u32 {
    if total_mb == 0 {
        return 4096;
    }
    // Half-gigabyte steps: the emulator takes whole megabytes, and a plan that
    // reads "5.3 GB" looks like a measurement rather than a choice.
    let third = (total_mb / 3).min(u32::MAX as u64) as u32;
    let rounded = (third / 512) * 512;
    let floor = recommended_ram_mb(total_mb, available_mb);
    rounded.clamp(floor, MAX_RAM_MB)
}

/// Build a preset plan from real host facts.
///
/// `Compatibility` deliberately asks for software rendering: it is slower, but
/// it is the mode that survives driver and EGL differences, and the UI must not
/// call it hardware acceleration afterwards.
pub fn preset_plan(host: &HostFacts, preset: Preset) -> HardwarePlan {
    let balanced = recommend(host);
    match preset {
        Preset::Balanced => HardwarePlan {
            preset: Some(Preset::Balanced.as_str().to_string()),
            reason: format!(
                "Balanced: {} of {} logical processors, {:.1} GB, {}. The automatic recommendation.",
                balanced.cores,
                host.logical_cpus,
                balanced.ram_mb as f32 / 1024.0,
                balanced.gpu.as_label()
            ),
            ..balanced
        },
        Preset::Performance => {
            let cores = performance_cores(host.logical_cpus);
            let ram_mb = performance_ram_mb(host.total_ram_mb, host.available_ram_mb);
            HardwarePlan {
                mode: PlanMode::Custom,
                cores,
                ram_mb,
                gpu: GpuMode::Host,
                preset: Some(Preset::Performance.as_str().to_string()),
                reason: format!(
                    "Performance: {cores} of {} logical processors and {:.1} GB, keeping the host GPU. Windows keeps the other threads.",
                    host.logical_cpus, ram_mb as f32 / 1024.0
                ),
            }
        }
        Preset::Compatibility => HardwarePlan {
                mode: PlanMode::Custom,
                gpu: GpuMode::Software,
                preset: Some(Preset::Compatibility.as_str().to_string()),
                reason: format!(
                    "Compatibility: {} vCPUs, {:.1} GB and software rendering (SwiftShader) — slower, but the rendering path that depends least on the host driver.",
                    balanced.cores, balanced.ram_mb as f32 / 1024.0
                ),
                ..balanced
            },
    }
}

pub fn validate_cores(cores: u32) -> Result<u32, String> {
    if ALLOWED_CORES.contains(&cores) {
        return Ok(cores);
    }
    if cores > MAX_CORES {
        return Err(format!(
            "The Android Emulator supports at most {MAX_CORES} vCPUs on this build and caps a larger request without telling Android; {cores} was not saved."
        ));
    }
    Err(format!(
        "Android can be given {} vCPUs; {} is not one of them.",
        ALLOWED_CORES
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(", "),
        cores
    ))
}

pub fn clamp_ram(ram_mb: u32) -> u32 {
    ram_mb.clamp(MIN_RAM_MB, MAX_RAM_MB)
}

/// Read the saved plan. A stored custom plan with an impossible value falls back
/// to the recommended one and says so, rather than booting a broken emulator.
pub fn load(store: &Store, host: &HostFacts) -> HardwarePlan {
    let recommended = recommend(host);
    let preset = store
        .setting(KEY_PRESET)
        .and_then(|value| Preset::parse(&value))
        .map(|preset| preset.as_str().to_string());
    let custom = store.setting(KEY_MODE).as_deref() == Some("custom");
    if !custom {
        return HardwarePlan {
            preset: preset.filter(|name| name == Preset::Balanced.as_str()),
            ..recommended
        };
    }

    let mut notes: Vec<String> = Vec::new();
    let cores = store
        .setting(KEY_CORES)
        .and_then(|v| v.parse::<u32>().ok())
        .and_then(|v| match validate_cores(v) {
            Ok(v) => Some(v),
            Err(err) => {
                notes.push(err);
                None
            }
        })
        .unwrap_or(recommended.cores);
    let ram_mb = store
        .setting(KEY_RAM)
        .and_then(|v| v.parse::<u32>().ok())
        .map(|v| {
            let clamped = clamp_ram(v);
            if clamped != v {
                notes.push(format!("{v} MB is outside 2–8 GB; using {clamped} MB."));
            }
            clamped
        })
        .unwrap_or(recommended.ram_mb);
    let gpu = store
        .setting(KEY_GPU)
        .and_then(|v| GpuMode::parse(&v))
        .unwrap_or(recommended.gpu);

    HardwarePlan {
        mode: PlanMode::Custom,
        cores,
        ram_mb,
        gpu,
        preset,
        reason: if notes.is_empty() {
            format!(
                "Custom: {cores} vCPUs, {:.1} GB, {}.",
                ram_mb as f32 / 1024.0,
                gpu.as_label()
            )
        } else {
            format!(
                "Custom: {cores} vCPUs, {:.1} GB, {}. {}",
                ram_mb as f32 / 1024.0,
                gpu.as_label(),
                notes.join(" ")
            )
        },
    }
}

pub fn persist(store: &Store, plan: &HardwarePlan) -> DwResult<()> {
    let mode = match plan.mode {
        PlanMode::Auto => "auto",
        PlanMode::Custom => "custom",
    };
    for (key, value) in [
        (KEY_MODE, mode.to_string()),
        (KEY_CORES, plan.cores.to_string()),
        (KEY_RAM, plan.ram_mb.to_string()),
        (KEY_GPU, plan.gpu.as_arg().to_string()),
        (
            KEY_PRESET,
            plan.preset.clone().unwrap_or_else(|| "custom".to_string()),
        ),
    ] {
        store.set_setting(key, &value)?;
    }
    Ok(())
}

/// Build and save a custom plan from a request. Everything is validated before
/// it is written, so the store never holds a value the emulator would reject.
pub fn custom_plan(
    host: &HostFacts,
    cores: u32,
    ram_mb: u32,
    gpu: GpuMode,
) -> Result<HardwarePlan, DwError> {
    let cores = validate_cores(cores).map_err(|message| {
        DwError::new(ErrorCode::InvalidConfig, message).detail("Choose 2, 4 or 6 vCPUs.")
    })?;
    let ram_mb = clamp_ram(ram_mb);
    let _ = host;
    Ok(HardwarePlan {
        mode: PlanMode::Custom,
        cores,
        ram_mb,
        gpu,
        reason: format!(
            "Custom: {cores} vCPUs, {:.1} GB, {}.",
            ram_mb as f32 / 1024.0,
            gpu.as_label()
        ),
        preset: None,
    })
}

/// Read the emulator's `-accel-check` report.
///
/// It prints `accel:`, then a numeric status (`0` = usable), then a sentence.
/// The number is authoritative; the sentence is shown to the user because it
/// names WHPX, HAXM or the conflicting hypervisor.
pub fn accel_usable(report: &str) -> bool {
    let mut lines = report.lines().map(str::trim).filter(|l| !l.is_empty());
    let mut after_header = false;
    for line in lines.by_ref() {
        if line == "accel:" {
            after_header = true;
            continue;
        }
        if after_header {
            if let Ok(code) = line.parse::<i32>() {
                return code == 0;
            }
        }
        if let Some(rest) = line.strip_prefix("accel:") {
            if let Some(code) = rest
                .split_whitespace()
                .next()
                .and_then(|token| token.parse::<i32>().ok())
            {
                return code == 0;
            }
        }
    }
    // Builds that only print the sentence still answer the question.
    report.contains("is installed and usable")
}

/// Whether a GLES renderer string names real GPU hardware. SwiftShader is the
/// only software backend the emulator reports, so it is the one negative.
pub fn hardware_gpu(renderer: Option<&str>) -> Option<bool> {
    let renderer = renderer?;
    if renderer.trim().is_empty() {
        return None;
    }
    if renderer.contains("SwiftShader") {
        Some(false)
    } else if renderer.contains("OpenGL ES") {
        Some(true)
    } else {
        None
    }
}

/// `MemTotal: 4013348 kB` → 3919 MB (kB ÷ 1024, rounded).
pub fn parse_android_meminfo(text: &str) -> Option<u32> {
    parse_android_memory(text).map(|(total, _)| total)
}

/// Total and available memory Android reports, in megabytes. `MemAvailable` is
/// absent on very old kernels; it then stays 0 rather than being invented.
pub fn parse_android_memory(text: &str) -> Option<(u32, u32)> {
    fn kb(text: &str, key: &str) -> Option<u64> {
        text.lines()
            .find(|line| line.trim_start().starts_with(key))?
            .split_whitespace()
            .find_map(|part| part.parse::<u64>().ok())
    }
    let total = kb(text, "MemTotal:")? / 1024;
    let available = kb(text, "MemAvailable:").unwrap_or(0) / 1024;
    Some((total as u32, available as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(cpus: u32, total: u64, available: u64) -> HostFacts {
        HostFacts {
            logical_cpus: cpus,
            total_ram_mb: total,
            available_ram_mb: available,
            memory_detected: true,
        }
    }

    #[test]
    fn cores_leave_half_the_machine_for_windows() {
        assert_eq!(recommended_cores(4), 2);
        assert_eq!(recommended_cores(8), 4);
        assert_eq!(recommended_cores(12), 6);
        assert_eq!(recommended_cores(16), 6);
        assert_eq!(recommended_cores(32), 6);
    }

    #[test]
    fn ram_is_a_quarter_of_the_machine_between_two_and_six_gigabytes() {
        assert_eq!(recommended_ram_mb(8 * 1024, 6 * 1024), 2048);
        assert_eq!(recommended_ram_mb(16 * 1024, 12 * 1024), 4096);
        assert_eq!(recommended_ram_mb(32 * 1024, 24 * 1024), 6144);
        assert_eq!(recommended_ram_mb(64 * 1024, 40 * 1024), 6144);
    }

    #[test]
    fn only_a_real_memory_squeeze_lowers_the_allocation() {
        // 16 GB machine with 2.4 GB free: the emulator itself is probably using
        // the rest, so the quarter-of-RAM answer stands.
        assert_eq!(recommended_ram_mb(16 * 1024, 2_400), 4096);
        // Under an eighth of the machine free is a genuine squeeze.
        assert_eq!(recommended_ram_mb(16 * 1024, 1_500), MIN_RAM_MB);
    }

    #[test]
    fn the_default_plan_asks_for_the_host_gpu() {
        let plan = recommend(&host(16, 16 * 1024, 12 * 1024));
        assert_eq!(plan.mode, PlanMode::Auto);
        assert_eq!(plan.cores, 6);
        assert_eq!(plan.ram_mb, 4096);
        assert_eq!(plan.gpu, GpuMode::Host);
    }

    #[test]
    fn an_undetectable_memory_size_is_stated_not_invented() {
        let facts = HostFacts {
            logical_cpus: 8,
            total_ram_mb: 0,
            available_ram_mb: 0,
            memory_detected: false,
        };
        let plan = recommend(&facts);
        assert_eq!(plan.ram_mb, 4096);
        assert!(plan.reason.contains("could not be read"));
    }

    #[test]
    fn only_supported_vcpu_counts_are_accepted() {
        assert!(validate_cores(4).is_ok());
        assert!(validate_cores(6).is_ok());
        assert!(validate_cores(3).is_err());
        assert!(validate_cores(0).is_err());
        // Measured: the emulator caps `-cores 8` at 6 and says so in its log, so
        // 8 is refused with that reason instead of being saved as a promise.
        let error = validate_cores(8).unwrap_err();
        assert!(error.contains("at most 6 vCPUs"), "{error}");
        assert!(validate_cores(12).is_err());
    }

    #[test]
    fn memory_is_clamped_into_the_range_the_emulator_handles() {
        assert_eq!(clamp_ram(256), MIN_RAM_MB);
        assert_eq!(clamp_ram(32 * 1024), MAX_RAM_MB);
        assert_eq!(clamp_ram(4096), 4096);
    }

    #[test]
    fn gpu_modes_map_to_emulator_flags() {
        assert_eq!(GpuMode::Host.as_arg(), "host");
        assert_eq!(GpuMode::Software.as_arg(), "swiftshader_indirect");
        assert_eq!(
            GpuMode::parse("swiftshader_indirect"),
            Some(GpuMode::Software)
        );
        assert_eq!(GpuMode::parse("nonsense"), None);
    }

    #[test]
    fn the_emulator_acceleration_report_is_read_from_its_status_code() {
        assert!(accel_usable(
            "accel:\n0\nWHPX(10.0.26200) is installed and usable.\naccel"
        ));
        assert!(!accel_usable(
            "accel:\n1\nWHPX is not installed and no hypervisor is present.\naccel"
        ));
        assert!(accel_usable("accel: 0 WHPX is installed and usable."));
        assert!(accel_usable("WHPX(10.0.26200) is installed and usable."));
        assert!(!accel_usable(""));
        assert!(!accel_usable("permission denied"));
    }

    #[test]
    fn the_effective_renderer_distinguishes_hardware_from_software() {
        assert_eq!(
            hardware_gpu(Some(
                "Android Emulator OpenGL ES Translator (NVIDIA GeForce RTX 5060 Laptop GPU)"
            )),
            Some(true)
        );
        assert_eq!(
            hardware_gpu(Some(
                "Android Emulator OpenGL ES Translator (Google SwiftShader)"
            )),
            Some(false)
        );
        assert_eq!(hardware_gpu(None), None);
    }

    #[test]
    fn performance_never_takes_every_thread_or_asks_for_more_than_android_gets() {
        assert_eq!(performance_cores(4), 2);
        assert_eq!(performance_cores(8), 4);
        assert_eq!(performance_cores(16), MAX_CORES);
        assert_eq!(performance_cores(24), MAX_CORES);
        assert!(performance_cores(64) <= MAX_CORES);
    }

    #[test]
    fn performance_memory_is_roomier_but_bounded() {
        assert_eq!(performance_ram_mb(8 * 1024, 4 * 1024), 2560);
        assert_eq!(performance_ram_mb(16 * 1024, 12 * 1024), 5120);
        assert_eq!(performance_ram_mb(32 * 1024, 24 * 1024), MAX_RAM_MB);
        // Never below the automatic plan, and never invented when there is no
        // memory reading at all.
        assert!(performance_ram_mb(16 * 1024, 512) >= MIN_RAM_MB);
        assert_eq!(performance_ram_mb(0, 0), 4096);
    }

    #[test]
    fn presets_are_real_plans_and_say_what_they_change() {
        let facts = host(16, 16 * 1024, 12 * 1024);
        let balanced = preset_plan(&facts, Preset::Balanced);
        assert_eq!(balanced.cores, recommend(&facts).cores);
        assert_eq!(balanced.preset.as_deref(), Some("balanced"));
        assert!(balanced.reason.contains("Balanced"));

        let performance = preset_plan(&facts, Preset::Performance);
        assert_eq!(performance.cores, MAX_CORES);
        assert_eq!(performance.gpu, GpuMode::Host);
        assert_eq!(performance.mode, PlanMode::Custom);
        assert!(performance.reason.contains("Performance"));

        let compatibility = preset_plan(&facts, Preset::Compatibility);
        assert_eq!(compatibility.gpu, GpuMode::Software);
        assert!(compatibility.reason.contains("software rendering"));
    }

    #[test]
    fn a_preset_name_is_read_in_the_values_the_ui_sends() {
        assert_eq!(Preset::parse("balanced"), Some(Preset::Balanced));
        assert_eq!(Preset::parse(" Performance "), Some(Preset::Performance));
        assert_eq!(Preset::parse("compatibility"), Some(Preset::Compatibility));
        assert_eq!(Preset::parse("turbo"), None);
    }

    #[test]
    fn android_meminfo_is_read_in_megabytes() {
        let text = "MemTotal:        4013348 kB\nMemFree:  100 kB\n";
        assert_eq!(parse_android_meminfo(text), Some(3919));
    }

    #[test]
    fn available_memory_is_reported_when_android_offers_it() {
        let text = "MemTotal:        4013348 kB\nMemAvailable:    1234567 kB\n";
        assert_eq!(parse_android_memory(text), Some((3919, 1205)));
        assert_eq!(parse_android_memory("MemTotal: 2048 kB\n"), Some((2, 0)));
        assert_eq!(parse_android_memory(""), None);
    }
}
