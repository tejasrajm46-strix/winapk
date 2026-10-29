//! Minimal reader for Android's compiled binary XML (AXML).
//!
//! An APK's `AndroidManifest.xml` is not text: it is a chunked binary format with
//! a string pool and typed attribute records. We only need the handful of values
//! the installer shows, so this reads exactly those and rejects anything that
//! does not satisfy the framing rules. Every read is bounds-checked against the
//! supplied buffer — the input is untrusted.

const RES_STRING_POOL: u16 = 0x0001;
const RES_XML_TYPE: u16 = 0x0003;
const RES_XML_START_ELEMENT: u16 = 0x0102;

const TYPE_REFERENCE: u8 = 0x01;
const TYPE_STRING: u8 = 0x03;
const TYPE_INT_DEC: u8 = 0x10;
const TYPE_INT_HEX: u8 = 0x11;

const NO_ENTRY: u32 = 0xFFFF_FFFF;

/// The manifest's `android:screenOrientation` declaration, reduced to the two
/// answers a display surface can be built from.
///
/// Android's enum has fifteen values. `sensor`, `user`, `fullSensor`, `behind`
/// and `nosensor` let the activity handle both orientations, so they are `None`:
/// claiming one of them would be inventing a constraint the app did not declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenOrientation {
    Portrait,
    Landscape,
}

impl ScreenOrientation {
    /// `android:screenOrientation` as an enum value (0 = landscape, 1 = portrait,
    /// 6/8/11 = landscape variants, 7/9/12 = portrait variants).
    pub fn from_enum(value: u32) -> Option<Self> {
        match value {
            0 | 6 | 8 | 11 => Some(ScreenOrientation::Landscape),
            1 | 7 | 9 | 12 => Some(ScreenOrientation::Portrait),
            _ => None,
        }
    }

    fn from_name(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "landscape" | "sensorlandscape" | "reverselandscape" | "userlandscape" => {
                Some(ScreenOrientation::Landscape)
            }
            "portrait" | "sensorportrait" | "reverseportrait" | "userportrait" => {
                Some(ScreenOrientation::Portrait)
            }
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ScreenOrientation::Portrait => "portrait",
            ScreenOrientation::Landscape => "landscape",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestAttrs {
    pub package: Option<String>,
    pub version_name: Option<String>,
    pub version_code: Option<u64>,
    pub min_sdk: Option<u32>,
    pub target_sdk: Option<u32>,
    /// A literal `android:label`. Resource references are reported separately.
    pub label: Option<String>,
    pub label_is_reference: bool,
    /// Set on a split APK (`split="config.x86_64"`), absent on the base.
    pub split: Option<String>,
    /// The orientation the app is drawn in: the launchable activity's
    /// declaration first, then the application's. `None` when neither locks one.
    pub orientation: Option<ScreenOrientation>,
    /// The launchable activity's class name, when the manifest names one. Shown
    /// as evidence for the orientation that was chosen.
    pub launcher_activity: Option<String>,
}

/// One `<activity>`/`<service>`/`<receiver>`/`<provider>` element as it is read.
/// Child elements (`intent-filter`, `action`, `category`) belong to the last one
/// seen, which is how a launcher activity is identified without a full tree.
#[derive(Default)]
struct Container {
    name: Option<String>,
    orientation: Option<ScreenOrientation>,
    has_main_action: bool,
    has_launcher_category: bool,
    is_activity: bool,
}

/// Everything a single element's attributes can add to.
#[derive(Default)]
struct ManifestState {
    attrs: ManifestAttrs,
    containers: Vec<Container>,
    application_orientation: Option<ScreenOrientation>,
}

struct Buf<'a>(&'a [u8]);

impl<'a> Buf<'a> {
    fn u8(&self, at: usize) -> Result<u8, String> {
        self.0
            .get(at)
            .copied()
            .ok_or_else(|| format!("read past end at {at}"))
    }

    fn u16(&self, at: usize) -> Result<u16, String> {
        let s = self
            .0
            .get(at..at + 2)
            .ok_or_else(|| format!("read past end at {at}"))?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }

    fn u32(&self, at: usize) -> Result<u32, String> {
        let s = self
            .0
            .get(at..at + 4)
            .ok_or_else(|| format!("read past end at {at}"))?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn bytes(&self, at: usize, len: usize) -> Result<&'a [u8], String> {
        self.0
            .get(at..at.checked_add(len).ok_or("length overflow")?)
            .ok_or_else(|| format!("read past end at {at}+{len}"))
    }
}

/// Parse `AndroidManifest.xml`. Returns the attributes we display; anything that
/// is absent in the file stays `None` rather than being defaulted to a guess.
pub fn parse_manifest(bytes: &[u8]) -> Result<ManifestAttrs, String> {
    let buf = Buf(bytes);
    if bytes.len() < 8 {
        return Err("manifest is shorter than a chunk header".into());
    }
    if buf.u16(0)? != RES_XML_TYPE {
        return Err(format!("unexpected root chunk 0x{:04x}", buf.u16(0)?));
    }
    let root_header = buf.u16(2)? as usize;
    let root_size = buf.u32(4)? as usize;
    if root_header < 8 || root_size < root_header || root_size > bytes.len() {
        return Err("malformed root chunk".into());
    }

    let mut strings: Vec<Option<String>> = Vec::new();
    let mut state = ManifestState::default();

    let mut pos = root_header;
    while pos + 8 <= root_size {
        let chunk_type = buf.u16(pos)?;
        let header_size = buf.u16(pos + 2)? as usize;
        let chunk_size = buf.u32(pos + 4)? as usize;
        if chunk_size < 8
            || header_size < 8
            || header_size > chunk_size
            || pos + chunk_size > root_size
        {
            return Err(format!("malformed chunk at {pos}"));
        }

        match chunk_type {
            RES_STRING_POOL => {
                strings = read_string_pool(&buf, pos, header_size, chunk_size)?;
            }
            RES_XML_START_ELEMENT => {
                read_element(&buf, pos, header_size, chunk_size, &strings, &mut state)?;
            }
            _ => {}
        }
        pos += chunk_size;
    }

    // The activity Android starts from the launcher is the one whose orientation
    // matters for the surface an app window is drawn on. The application-level
    // declaration is the documented fallback.
    let mut out = state.attrs;
    if let Some(launcher) = state
        .containers
        .iter()
        .find(|c| c.is_activity && c.has_main_action && c.has_launcher_category)
    {
        out.launcher_activity = launcher.name.clone();
        out.orientation = launcher.orientation;
    }
    if out.orientation.is_none() {
        out.orientation = state.application_orientation;
    }

    Ok(out)
}

fn is_container(element: &str) -> bool {
    matches!(
        element,
        "activity" | "activity-alias" | "service" | "receiver" | "provider"
    )
}

fn read_string_pool(
    buf: &Buf,
    start: usize,
    header_size: usize,
    chunk_size: usize,
) -> Result<Vec<Option<String>>, String> {
    if header_size < 28 {
        return Err("string pool header too small".into());
    }
    let count = buf.u32(start + 8)? as usize;
    let flags = buf.u32(start + 16)?;
    let strings_start = buf.u32(start + 20)? as usize;
    let utf8 = flags & 0x100 != 0;
    let chunk_end = start + chunk_size;
    let data_start = start
        .checked_add(strings_start)
        .ok_or("string pool overflow")?;
    if data_start > chunk_end {
        return Err("string pool data start outside chunk".into());
    }
    if header_size + count * 4 > chunk_size {
        return Err("string offsets exceed chunk".into());
    }

    let mut out = Vec::with_capacity(count.min(4096));
    for i in 0..count {
        let offset = buf.u32(start + header_size + i * 4)? as usize;
        let at = data_start
            .checked_add(offset)
            .ok_or("string offset overflow")?;
        if at >= chunk_end {
            out.push(None);
            continue;
        }
        out.push(read_string(buf, at, chunk_end, utf8).ok());
    }
    Ok(out)
}

fn read_string(buf: &Buf, at: usize, end: usize, utf8: bool) -> Result<String, String> {
    if utf8 {
        let first = buf.u8(at)?;
        let (len, mut p) = if first & 0x80 != 0 {
            let second = buf.u8(at + 1)?;
            ((((first & 0x7f) as usize) << 8) | second as usize, at + 2)
        } else {
            (first as usize, at + 1)
        };
        if p + len > end {
            return Err("utf8 string past chunk".into());
        }
        let raw = buf.bytes(p, len)?;
        p += len;
        // Trailing NUL is part of the record but not the value.
        let _ = p;
        std::str::from_utf8(raw)
            .map(|s| s.to_string())
            .map_err(|e| e.to_string())
    } else {
        let first = buf.u16(at)?;
        let (len, mut p) = if first & 0x8000 != 0 {
            let second = buf.u16(at + 2)?;
            (
                (((first & 0x7fff) as usize) << 16) | second as usize,
                at + 4,
            )
        } else {
            (first as usize, at + 2)
        };
        if p + len * 2 > end {
            return Err("utf16 string past chunk".into());
        }
        let mut units = Vec::with_capacity(len);
        for _ in 0..len {
            units.push(buf.u16(p)?);
            p += 2;
        }
        Ok(String::from_utf16_lossy(&units))
    }
}

fn read_element(
    buf: &Buf,
    start: usize,
    header_size: usize,
    _chunk_size: usize,
    strings: &[Option<String>],
    state: &mut ManifestState,
) -> Result<(), String> {
    let name_atom = buf.u32(start + 20)?;
    let attr_start = buf.u16(start + 24)? as usize;
    let attr_size = buf.u16(start + 26)? as usize;
    let attr_count = buf.u16(start + 28)? as usize;

    let Some(element) = resolve(strings, name_atom) else {
        return Ok(());
    };
    if attr_count == 0 || attr_size < 20 {
        return Ok(());
    }

    if is_container(&element) {
        state.containers.push(Container {
            is_activity: matches!(element.as_str(), "activity" | "activity-alias"),
            ..Container::default()
        });
    }
    let out = &mut state.attrs;
    let containers = &mut state.containers;
    let application_orientation = &mut state.application_orientation;

    let attrs_base = start
        .checked_add(header_size)
        .and_then(|v| v.checked_add(attr_start))
        .ok_or("attribute offset overflow")?;

    for i in 0..attr_count {
        let a = attrs_base + i * attr_size;
        let attr_name = resolve(strings, buf.u32(a + 4)?);
        let raw_value = buf.u32(a + 8)?;
        let data_type = buf.u8(a + 15)?;
        let data = buf.u32(a + 16)?;

        let Some(attr_name) = attr_name else { continue };
        let value = Value::new(strings, raw_value, data_type, data);

        match (element.as_str(), attr_name.as_str()) {
            ("manifest", "package") => out.package = value.as_string(),
            ("manifest", "versionName") => out.version_name = value.as_string(),
            ("manifest", "versionCode") => out.version_code = value.as_u32().map(u64::from),
            ("manifest", "split") => out.split = value.as_string(),
            ("uses-sdk", "minSdkVersion") => out.min_sdk = value.as_sdk(),
            ("uses-sdk", "targetSdkVersion") => out.target_sdk = value.as_sdk(),
            ("application", "label") => match value {
                Value::Str(s) => out.label = Some(s),
                Value::Reference => out.label_is_reference = true,
                _ => {}
            },
            ("application", "screenOrientation") => {
                if let Some(orientation) = value.as_orientation() {
                    *application_orientation = Some(orientation);
                }
            }
            ("action", "name") => {
                if value.as_string().as_deref() == Some("android.intent.action.MAIN") {
                    if let Some(container) = containers.last_mut() {
                        container.has_main_action = true;
                    }
                }
            }
            ("category", "name") => {
                if value.as_string().as_deref() == Some("android.intent.category.LAUNCHER") {
                    if let Some(container) = containers.last_mut() {
                        container.has_launcher_category = true;
                    }
                }
            }
            (_, "screenOrientation") => {
                if let Some(orientation) = value.as_orientation() {
                    if let Some(container) = containers.last_mut() {
                        container.orientation = Some(orientation);
                    }
                }
            }
            (_, "name") => {
                if let Some(name) = value.as_string() {
                    if let Some(container) = containers.last_mut() {
                        container.name = Some(name);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn resolve(strings: &[Option<String>], atom: u32) -> Option<String> {
    if atom == NO_ENTRY {
        return None;
    }
    strings.get(atom as usize).cloned().flatten()
}

enum Value {
    Str(String),
    Int(u32),
    Reference,
    Other,
}

impl Value {
    fn new(strings: &[Option<String>], raw: u32, data_type: u8, data: u32) -> Self {
        if raw != NO_ENTRY {
            if let Some(s) = resolve(strings, raw) {
                return Value::Str(s);
            }
        }
        match data_type {
            TYPE_STRING => match resolve(strings, data) {
                Some(s) => Value::Str(s),
                None => Value::Other,
            },
            TYPE_INT_DEC | TYPE_INT_HEX => Value::Int(data),
            TYPE_REFERENCE => Value::Reference,
            _ => Value::Other,
        }
    }

    fn as_string(&self) -> Option<String> {
        match self {
            Value::Str(s) if !s.is_empty() => Some(s.clone()),
            Value::Int(i) => Some(i.to_string()),
            _ => None,
        }
    }

    fn as_u32(&self) -> Option<u32> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Str(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// `minSdkVersion` / `targetSdkVersion` may be an int or a codename string.
    fn as_sdk(&self) -> Option<u32> {
        self.as_u32()
    }

    /// `screenOrientation` is an enum (an int), but a hand-written manifest can
    /// name it as text; both forms are read the same way.
    fn as_orientation(&self) -> Option<ScreenOrientation> {
        match self {
            Value::Int(value) => ScreenOrientation::from_enum(*value),
            Value::Str(name) => ScreenOrientation::from_name(name),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- tiny AXML writer, only for tests -------------------------------------

    fn utf16_pool(strings: &[&str]) -> Vec<u8> {
        let mut data = Vec::new();
        let mut offsets = Vec::new();
        for s in strings {
            offsets.push(data.len() as u32);
            let units: Vec<u16> = s.encode_utf16().collect();
            data.extend_from_slice(&(units.len() as u16).to_le_bytes());
            for u in units {
                data.extend_from_slice(&u.to_le_bytes());
            }
            data.extend_from_slice(&0u16.to_le_bytes());
        }
        let header_size = 28usize;
        let strings_start = header_size + offsets.len() * 4;
        let size = strings_start + data.len();
        let mut out = Vec::new();
        out.extend_from_slice(&RES_STRING_POOL.to_le_bytes());
        out.extend_from_slice(&(header_size as u16).to_le_bytes());
        out.extend_from_slice(&(size as u32).to_le_bytes());
        out.extend_from_slice(&(strings.len() as u32).to_le_bytes()); // stringCount
        out.extend_from_slice(&0u32.to_le_bytes()); // styleCount
        out.extend_from_slice(&0u32.to_le_bytes()); // flags: UTF-16
        out.extend_from_slice(&(strings_start as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // stylesStart
        for o in offsets {
            out.extend_from_slice(&o.to_le_bytes());
        }
        out.extend_from_slice(&data);
        out
    }

    struct Attr<'a> {
        name: &'a str,
        value: AttrValue<'a>,
    }

    enum AttrValue<'a> {
        Str(&'a str),
        Int(u32),
        Ref,
    }

    fn start_element(strings: &mut Vec<&'static str>, element: &str, attrs: &[Attr]) -> Vec<u8> {
        let element_atom = intern(strings, element);
        let mut attr_bytes = Vec::new();
        for a in attrs {
            let name_atom = intern(strings, a.name);
            let (raw, data_type, data) = match a.value {
                AttrValue::Str(s) => (intern(strings, s), TYPE_STRING, intern(strings, s)),
                AttrValue::Int(i) => (NO_ENTRY, TYPE_INT_DEC, i),
                AttrValue::Ref => (NO_ENTRY, TYPE_REFERENCE, 0x7f010001),
            };
            attr_bytes.extend_from_slice(&NO_ENTRY.to_le_bytes()); // ns
            attr_bytes.extend_from_slice(&name_atom.to_le_bytes());
            attr_bytes.extend_from_slice(&raw.to_le_bytes());
            attr_bytes.extend_from_slice(&20u16.to_le_bytes()); // size
            attr_bytes.push(0); // res0
            attr_bytes.push(data_type);
            attr_bytes.extend_from_slice(&data.to_le_bytes());
        }
        let header_size = 16usize;
        let attr_start = 20usize;
        let size = header_size + attr_start + attr_bytes.len();
        let mut out = Vec::new();
        out.extend_from_slice(&RES_XML_START_ELEMENT.to_le_bytes());
        out.extend_from_slice(&(header_size as u16).to_le_bytes());
        out.extend_from_slice(&(size as u32).to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes()); // line
        out.extend_from_slice(&NO_ENTRY.to_le_bytes()); // comment
        out.extend_from_slice(&NO_ENTRY.to_le_bytes()); // ns
        out.extend_from_slice(&element_atom.to_le_bytes());
        out.extend_from_slice(&(attr_start as u16).to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&(attrs.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&attr_bytes);
        out
    }

    fn intern(strings: &mut Vec<&'static str>, value: &str) -> u32 {
        // Leak is fine in a test; keeps lifetimes simple.
        let boxed: &'static str = Box::leak(value.to_string().into_boxed_str());
        if let Some(pos) = strings.iter().position(|s| *s == boxed) {
            return pos as u32;
        }
        strings.push(boxed);
        (strings.len() - 1) as u32
    }

    fn wrap(chunks: Vec<u8>) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&RES_XML_TYPE.to_le_bytes());
        out.extend_from_slice(&8u16.to_le_bytes());
        let size = 8 + chunks.len();
        out.extend_from_slice(&(size as u32).to_le_bytes());
        out.extend_from_slice(&chunks);
        out
    }

    #[test]
    fn reads_core_manifest_values() {
        let mut strings: Vec<&'static str> = Vec::new();
        // Build elements first so the string pool is complete, then emit it first.
        let manifest = start_element(
            &mut strings,
            "manifest",
            &[
                Attr {
                    name: "package",
                    value: AttrValue::Str("com.example.demo"),
                },
                Attr {
                    name: "versionCode",
                    value: AttrValue::Int(42),
                },
                Attr {
                    name: "versionName",
                    value: AttrValue::Str("1.2.3"),
                },
            ],
        );
        let uses_sdk = start_element(
            &mut strings,
            "uses-sdk",
            &[
                Attr {
                    name: "minSdkVersion",
                    value: AttrValue::Int(26),
                },
                Attr {
                    name: "targetSdkVersion",
                    value: AttrValue::Int(34),
                },
            ],
        );
        let application = start_element(
            &mut strings,
            "application",
            &[Attr {
                name: "label",
                value: AttrValue::Str("Demo App"),
            }],
        );

        let mut chunks = utf16_pool(&strings);
        chunks.extend_from_slice(&manifest);
        chunks.extend_from_slice(&uses_sdk);
        chunks.extend_from_slice(&application);

        let parsed = parse_manifest(&wrap(chunks)).expect("parses");
        assert_eq!(parsed.package.as_deref(), Some("com.example.demo"));
        assert_eq!(parsed.version_code, Some(42));
        assert_eq!(parsed.version_name.as_deref(), Some("1.2.3"));
        assert_eq!(parsed.min_sdk, Some(26));
        assert_eq!(parsed.target_sdk, Some(34));
        assert_eq!(parsed.label.as_deref(), Some("Demo App"));
        assert!(!parsed.label_is_reference);
    }

    #[test]
    fn resource_reference_label_is_not_invented() {
        let mut strings: Vec<&'static str> = Vec::new();
        let manifest = start_element(
            &mut strings,
            "manifest",
            &[Attr {
                name: "package",
                value: AttrValue::Str("com.example.ref"),
            }],
        );
        let application = start_element(
            &mut strings,
            "application",
            &[Attr {
                name: "label",
                value: AttrValue::Ref,
            }],
        );
        let mut chunks = utf16_pool(&strings);
        chunks.extend_from_slice(&manifest);
        chunks.extend_from_slice(&application);

        let parsed = parse_manifest(&wrap(chunks)).expect("parses");
        assert_eq!(parsed.label, None);
        assert!(parsed.label_is_reference);
    }

    #[test]
    fn rejects_wrong_root_chunk() {
        let mut bytes = wrap(Vec::new());
        bytes[0] = 0x09;
        assert!(parse_manifest(&bytes).is_err());
    }

    #[test]
    fn rejects_truncated_buffer() {
        let mut strings: Vec<&'static str> = Vec::new();
        let manifest = start_element(
            &mut strings,
            "manifest",
            &[Attr {
                name: "package",
                value: AttrValue::Str("com.example.cut"),
            }],
        );
        let mut chunks = utf16_pool(&strings);
        chunks.extend_from_slice(&manifest);
        let full = wrap(chunks);
        for cut in [0, 4, 9, 20, full.len() - 1] {
            let truncated = &full[..cut.min(full.len())];
            assert!(
                parse_manifest(truncated).is_err(),
                "cut {cut} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_absurd_chunk_sizes() {
        let mut bytes = wrap(Vec::new());
        bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_manifest(&bytes).is_err());
    }

    #[test]
    fn empty_manifest_yields_no_fields() {
        let parsed = parse_manifest(&wrap(Vec::new())).expect("empty root parses");
        assert_eq!(parsed, ManifestAttrs::default());
    }

    /// A manifest with one launcher activity. `orientation` is the enum value
    /// Android stores for `android:screenOrientation` (0 = landscape,
    /// 1 = portrait, 4 = sensor).
    fn manifest_with_activity(
        activity_orientation: Option<u32>,
        app_orientation: Option<u32>,
    ) -> Vec<u8> {
        let mut strings: Vec<&'static str> = Vec::new();
        let manifest = start_element(
            &mut strings,
            "manifest",
            &[Attr {
                name: "package",
                value: AttrValue::Str("org.example.game"),
            }],
        );
        let application = start_element(
            &mut strings,
            "application",
            &match app_orientation {
                Some(value) => vec![Attr {
                    name: "screenOrientation",
                    value: AttrValue::Int(value),
                }],
                None => Vec::new(),
            },
        );
        let mut activity_attrs = vec![Attr {
            name: "name",
            value: AttrValue::Str("org.example.game.MainActivity"),
        }];
        if let Some(value) = activity_orientation {
            activity_attrs.push(Attr {
                name: "screenOrientation",
                value: AttrValue::Int(value),
            });
        }
        let activity = start_element(&mut strings, "activity", &activity_attrs);
        let intent_filter = start_element(&mut strings, "intent-filter", &[]);
        let action = start_element(
            &mut strings,
            "action",
            &[Attr {
                name: "name",
                value: AttrValue::Str("android.intent.action.MAIN"),
            }],
        );
        let category = start_element(
            &mut strings,
            "category",
            &[Attr {
                name: "name",
                value: AttrValue::Str("android.intent.category.LAUNCHER"),
            }],
        );
        let mut chunks = utf16_pool(&strings);
        for chunk in [
            manifest,
            application,
            activity,
            intent_filter,
            action,
            category,
        ] {
            chunks.extend_from_slice(&chunk);
        }
        wrap(chunks)
    }

    #[test]
    fn the_launcher_activitys_declared_orientation_is_read() {
        // Block'Buster declares screenOrientation=0 on its only activity.
        let parsed = parse_manifest(&manifest_with_activity(Some(0), None)).expect("parses");
        assert_eq!(parsed.orientation, Some(ScreenOrientation::Landscape));
        assert_eq!(
            parsed.launcher_activity.as_deref(),
            Some("org.example.game.MainActivity")
        );

        let portrait = parse_manifest(&manifest_with_activity(Some(1), None)).expect("parses");
        assert_eq!(portrait.orientation, Some(ScreenOrientation::Portrait));
    }

    #[test]
    fn a_sensor_activity_does_not_claim_a_fixed_orientation() {
        // 4 = sensor: the app handles both orientations, so no surface is
        // "correct" and DroidWin must not pretend the manifest asked for one.
        let parsed = parse_manifest(&manifest_with_activity(Some(4), None)).expect("parses");
        assert_eq!(parsed.orientation, None);
    }

    #[test]
    fn the_application_orientation_is_the_fallback() {
        let parsed = parse_manifest(&manifest_with_activity(None, Some(0))).expect("parses");
        assert_eq!(parsed.orientation, Some(ScreenOrientation::Landscape));

        // A launcher that declares its own wins over the application's.
        let both = parse_manifest(&manifest_with_activity(Some(1), Some(0))).expect("parses");
        assert_eq!(both.orientation, Some(ScreenOrientation::Portrait));
    }

    #[test]
    fn the_orientation_enum_matches_the_documented_values() {
        assert_eq!(
            ScreenOrientation::from_enum(0),
            Some(ScreenOrientation::Landscape)
        );
        assert_eq!(
            ScreenOrientation::from_enum(6),
            Some(ScreenOrientation::Landscape)
        );
        assert_eq!(
            ScreenOrientation::from_enum(9),
            Some(ScreenOrientation::Portrait)
        );
        assert_eq!(ScreenOrientation::from_enum(2), None); // user
        assert_eq!(ScreenOrientation::from_enum(10), None); // fullSensor
        assert_eq!(ScreenOrientation::from_enum(14), None); // locked
    }
}
