//! Device and app selection for the interactive commands: `golem tree`,
//! `golem probe`, sessions and the MCP `devices` tool.
//!
//! `golem tree` and `golem probe` read the screen of a device that runs:
//! when the request matches more than one, they fail with the candidates
//! instead of picking one. A session picks as a flow's device constraint
//! does, and may boot a device ([`choose_for_session`]).

use std::fmt::Write;

use anyhow::{bail, Context, Result};
use golem_devices::{DeviceInfo, DeviceState, DeviceType, OsVersionSpec, Platform};
use golem_driver::CompanionHealth;
use golem_parser::ProjectAppConfig;

/// What the caller asked for. Every field is optional.
#[derive(Debug, Clone, Default)]
pub struct TargetQuery {
    /// The OS, as a flow's `os`: `ios`, `ios:26`, `ios:latest`.
    pub os: Option<OsQuery>,
    /// The form factor, as a flow's `type`.
    pub device_type: Option<DeviceType>,
    /// UDID, serial or name.
    pub device: Option<String>,
    /// Bundle id, used as given.
    pub bundle: Option<String>,
    /// An app name from the project `[[apps]]` registry.
    pub app: Option<String>,
}

/// An `os` value: a platform, and the versions it allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsQuery {
    pub platform: Platform,
    /// `None` for a bare platform: any version.
    pub version: Option<OsVersionSpec>,
    /// As the caller wrote it.
    pub text: String,
}

impl OsQuery {
    /// Read `ios`, `android`, `ios:26`, `ios:26+` or `ios:latest`. A
    /// session holds one device, so `latest:N` is refused.
    pub fn parse(text: &str) -> Result<OsQuery> {
        let platform = |p: &str| match p {
            "ios" => Ok(Platform::Ios),
            "android" => Ok(Platform::Android),
            other => bail!(
                "unknown os: {other}. Use ios or android, with an optional :version, as in a flow"
            ),
        };
        let version = match text.split_once(':') {
            None => {
                return Ok(OsQuery {
                    platform: platform(text)?,
                    version: None,
                    text: text.to_string(),
                })
            }
            Some((p, _)) => {
                platform(p)?;
                golem_devices::version::parse_os_version(text)
                    .with_context(|| format!("invalid os {text:?}"))?
            }
        };
        if let OsVersionSpec::Latest { count, .. } = version {
            if count > 1 {
                bail!("os {text:?} asks for {count} versions; a session holds one device");
            }
        }
        let (OsVersionSpec::Exact { platform, .. }
        | OsVersionSpec::Minimum { platform, .. }
        | OsVersionSpec::Latest { platform, .. }) = version;
        Ok(OsQuery {
            platform,
            version: Some(version),
            text: text.to_string(),
        })
    }
}

impl OsQuery {
    /// The query for a flow slot's platform and OS version.
    pub fn of(platform: Platform, version: Option<OsVersionSpec>) -> OsQuery {
        let text = match version {
            None => platform.to_string(),
            Some(OsVersionSpec::Exact { major, .. }) => format!("{platform}:{major}"),
            Some(OsVersionSpec::Minimum { major, .. }) => format!("{platform}:{major}+"),
            Some(OsVersionSpec::Latest { .. }) => format!("{platform}:latest"),
        };
        OsQuery {
            platform,
            version,
            text,
        }
    }
}

/// Read a flow-style `type`: `phone` or `tablet`.
pub fn parse_device_type(text: &str) -> Result<DeviceType> {
    match text {
        "phone" => Ok(DeviceType::Phone),
        "tablet" => Ok(DeviceType::Tablet),
        other => bail!("unknown type: {other}. Use phone or tablet"),
    }
}

impl TargetQuery {
    pub fn platform(&self) -> Option<Platform> {
        self.os.as_ref().map(|o| o.platform)
    }

    /// The query's `os` and `type`, for an error message.
    fn shape(&self) -> String {
        let mut parts = Vec::new();
        if let Some(os) = &self.os {
            parts.push(os.text.clone());
        }
        if let Some(t) = self.device_type {
            parts.push(t.to_string());
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("{} ", parts.join(" "))
        }
    }

    /// Whether `device` has the query's platform, OS version and type.
    /// `latest` is the newest OS major among `all`, in any state.
    pub fn fits(&self, device: &DeviceInfo, all: &[DeviceInfo]) -> bool {
        if let Some(os) = &self.os {
            if device.platform != os.platform {
                return false;
            }
            let ok = match os.version {
                None => true,
                Some(OsVersionSpec::Exact { major, .. }) => device.os_major == major,
                Some(OsVersionSpec::Minimum { major, .. }) => device.os_major >= major,
                Some(OsVersionSpec::Latest { platform, .. }) => {
                    all.iter()
                        .filter(|d| d.platform == platform)
                        .map(|d| d.os_major)
                        .max()
                        == Some(device.os_major)
                }
            };
            if !ok {
                return false;
            }
        }
        self.device_type.is_none_or(|t| device.device_type == t)
    }

    /// The device that `device` names among `devices`: an exact UDID or
    /// serial, then an exact name, then a substring of either, all
    /// case-insensitive. `Ok(None)` when nothing matches; an error when
    /// more than one device matches at the first step that matches.
    fn named<'a>(&self, devices: &[&'a DeviceInfo]) -> Result<Option<&'a DeviceInfo>> {
        let Some(wanted) = self.device.as_deref() else {
            return Ok(None);
        };
        let wanted_lc = wanted.to_lowercase();
        let steps: [&dyn Fn(&DeviceInfo) -> bool; 3] = [
            &|d| d.udid.to_lowercase() == wanted_lc,
            &|d| d.name.to_lowercase() == wanted_lc,
            &|d| {
                d.udid.to_lowercase().contains(&wanted_lc)
                    || d.name.to_lowercase().contains(&wanted_lc)
            },
        ];
        for matches_step in steps {
            let hits: Vec<&DeviceInfo> = devices
                .iter()
                .copied()
                .filter(|d| matches_step(d))
                .collect();
            match hits.as_slice() {
                [] => continue,
                [only] => return Ok(Some(only)),
                many => {
                    return Err(ambiguous(
                        &format!("{} devices match \"{wanted}\"", many.len()),
                        many,
                    ))
                }
            }
        }
        Ok(None)
    }
}

/// A device with a live companion, and the app to act on.
#[derive(Debug)]
pub struct Target {
    pub device: DeviceInfo,
    /// Empty when the request named no app and the registry has none to
    /// default to. On iOS the companion then reads the app it last
    /// launched; on Android the hierarchy covers the whole screen anyway.
    pub bundle: String,
    pub port: u16,
    pub health: CompanionHealth,
}

impl Target {
    /// A driver for this device and app, on the live companion.
    pub fn driver(&self) -> Box<dyn golem_driver::PlatformDriver> {
        let (udid, bundle) = (self.device.udid.clone(), self.bundle.clone());
        match self.device.platform {
            Platform::Android => Box::new(golem_driver::android::AndroidDriver::new(
                udid,
                bundle,
                self.port,
                self.device.physical,
            )),
            Platform::Ios => Box::new(golem_driver::ios::IosDriver::new(
                udid,
                bundle,
                self.port,
                self.device.physical,
            )),
        }
    }
}

/// A device in the `devices` listing.
#[derive(Debug, Clone)]
pub struct DeviceEntry {
    pub device: DeviceInfo,
    /// The port of a live companion on this device.
    pub companion_port: Option<u16>,
}

/// Whether golem can drive `device` now: a booted simulator or emulator,
/// or a connected physical device.
fn is_usable(device: &DeviceInfo) -> bool {
    matches!(device.state, DeviceState::Booted | DeviceState::Connected)
}

fn state_label(state: DeviceState) -> &'static str {
    match state {
        DeviceState::Booted => "booted",
        DeviceState::Shutdown => "shutdown",
        DeviceState::Connected => "connected",
        DeviceState::NeedsCreation => "needs-creation",
    }
}

fn candidate_line(d: &DeviceInfo) -> String {
    format!(
        "  {} {} \"{}\" os:{}",
        d.platform, d.udid, d.name, d.os_version
    )
}

/// `devices` in a stable order: discovery order varies between calls.
fn sorted<'a>(devices: &[&'a DeviceInfo]) -> Vec<&'a DeviceInfo> {
    let mut out = devices.to_vec();
    out.sort_by(|a, b| {
        (a.platform.to_string(), &a.name, &a.udid).cmp(&(b.platform.to_string(), &b.name, &b.udid))
    });
    out
}

/// `head`, then every candidate on its own line.
fn ambiguous(head: &str, matches: &[&DeviceInfo]) -> anyhow::Error {
    let mut msg = format!("{head}; set device to one of:");
    for d in sorted(matches) {
        msg.push('\n');
        msg.push_str(&candidate_line(d));
    }
    anyhow::anyhow!(msg)
}

/// Pick the one usable device the query names.
///
/// With no `device`, the single usable device (after the platform filter)
/// is the target. With a `device`, an exact UDID or serial wins, then an
/// exact name, then a substring of either; all case-insensitive. More than
/// one match at the first step that matches anything is an error listing
/// the candidates.
pub fn select_device<'a>(devices: &'a [DeviceInfo], query: &TargetQuery) -> Result<&'a DeviceInfo> {
    let on_platform: Vec<&DeviceInfo> = devices.iter().filter(|d| query.fits(d, devices)).collect();
    let usable: Vec<&DeviceInfo> = on_platform
        .iter()
        .copied()
        .filter(|d| is_usable(d))
        .collect();
    let platform = query.shape();

    let Some(wanted) = query.device.as_deref() else {
        return match usable.as_slice() {
            [only] => Ok(only),
            [] => bail!("no booted {platform}device; start a simulator or emulator first"),
            many => Err(ambiguous(
                &format!("{} booted {platform}devices", many.len()),
                many,
            )),
        };
    };

    if let Some(hit) = query.named(&usable)? {
        return Ok(hit);
    }
    let wanted_lc = wanted.to_lowercase();
    if let Some(idle) = on_platform
        .iter()
        .find(|d| d.udid.to_lowercase() == wanted_lc || d.name.to_lowercase() == wanted_lc)
    {
        bail!(
            "{} ({}) is {}; boot it first",
            idle.name,
            idle.udid,
            state_label(idle.state)
        );
    }
    let mut msg = format!("no booted {platform}device matches \"{wanted}\"");
    if !usable.is_empty() {
        msg.push_str("; booted devices:");
        for d in sorted(&usable) {
            msg.push('\n');
            msg.push_str(&candidate_line(d));
        }
    }
    bail!(msg)
}

/// The bundle id to act on: `bundle` as given, else the registry entry
/// `app` names, else the registry's only app. Empty when nothing names an
/// app and the registry holds none or several.
pub fn resolve_bundle(query: &TargetQuery, apps: &[ProjectAppConfig]) -> Result<String> {
    if let Some(bundle) = &query.bundle {
        return Ok(bundle.clone());
    }
    if let Some(name) = &query.app {
        let Some(app) = apps.iter().find(|a| &a.name == name) else {
            let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
            if names.is_empty() {
                bail!("unknown app \"{name}\": golem.toml has no [[apps]]; set bundle instead");
            }
            bail!(
                "unknown app \"{name}\"; golem.toml [[apps]] has: {}",
                names.join(", ")
            );
        };
        return match &app.bundle {
            Some(bundle) => Ok(bundle.clone()),
            None => bail!("app \"{name}\" in golem.toml has no bundle; set bundle instead"),
        };
    }
    match apps {
        [only] => Ok(only.bundle.clone().unwrap_or_default()),
        _ => Ok(String::new()),
    }
}

/// The device a session takes.
#[derive(Debug)]
pub enum Choice<'a> {
    /// Booted or connected: use it as it is.
    Ready(&'a DeviceInfo),
    /// Shut down: boot it first.
    Boot(&'a DeviceInfo),
}

/// Pick the device for a session, as a flow's device constraint would.
///
/// A named device is used in any state. Otherwise a free booted device
/// that fits wins, a simulator or emulator before a physical device, then
/// the newest OS. With none, the fitting shut-down device with the newest
/// OS is booted. `in_use` says whether a run or a session holds a device.
///
/// Unlike [`select_device`], several fitting devices are no error: a
/// session that may boot a device picks one as `golem run` does.
pub fn choose_for_session<'a>(
    devices: &'a [DeviceInfo],
    query: &TargetQuery,
    in_use: &dyn Fn(&DeviceInfo) -> bool,
) -> Result<Choice<'a>> {
    let fitting: Vec<&DeviceInfo> = devices.iter().filter(|d| query.fits(d, devices)).collect();
    let shape = query.shape();
    let ready_or_boot = |d: &'a DeviceInfo| match d.state {
        DeviceState::Shutdown => Ok(Choice::Boot(d)),
        DeviceState::NeedsCreation => bail!("{} ({}) does not exist yet", d.name, d.udid),
        _ => Ok(Choice::Ready(d)),
    };
    if let Some(wanted) = query.device.as_deref() {
        if let Some(hit) = query.named(&fitting)? {
            return ready_or_boot(hit);
        }
        let all: Vec<&DeviceInfo> = devices.iter().collect();
        if let Some(other) = query.named(&all)? {
            bail!(
                "{} ({}) is {} os:{}, not {}",
                other.name,
                other.udid,
                other.device_type,
                other.os_version,
                shape.trim_end()
            );
        }
        bail!("no {shape}device matches \"{wanted}\"");
    }
    let newest_first = |a: &&DeviceInfo, b: &&DeviceInfo| {
        (a.physical, std::cmp::Reverse(a.os_major), &a.name, &a.udid).cmp(&(
            b.physical,
            std::cmp::Reverse(b.os_major),
            &b.name,
            &b.udid,
        ))
    };
    let mut usable: Vec<&DeviceInfo> = fitting.iter().copied().filter(|d| is_usable(d)).collect();
    usable.sort_by(newest_first);
    if let Some(free) = usable.iter().find(|d| !in_use(d)) {
        return Ok(Choice::Ready(free));
    }
    if !usable.is_empty() {
        bail!(
            "every booted {shape}device is in use by a run or a session ({}); close one, or boot another",
            usable
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let mut shutdown: Vec<&DeviceInfo> = fitting
        .iter()
        .copied()
        .filter(|d| d.state == DeviceState::Shutdown)
        .collect();
    shutdown.sort_by(newest_first);
    match shutdown.first() {
        Some(d) => Ok(Choice::Boot(d)),
        None => bail!(
            "no {shape}device on this host; create a simulator in Xcode or an emulator in Android Studio"
        ),
    }
}

/// The device and app for a session: [`choose_for_session`], then a boot
/// when the choice needs one and `boot` allows it. A device golem boots is
/// marked so that the daemon shuts it down when it exits.
pub async fn select_for_session(
    query: &TargetQuery,
    apps: &[ProjectAppConfig],
    resource_mgr: &golem_devices::resource_manager::ResourceManager,
    boot: bool,
    on_boot: &(dyn Fn(&DeviceInfo) + Send + Sync),
) -> Result<Selection> {
    let bundle = resolve_bundle(query, apps)?;
    let devices = discover_devices(query.platform()).await;
    let in_use = |d: &DeviceInfo| resource_mgr.port_for(&d.udid).is_some();
    let device = match choose_for_session(&devices, query, &in_use)? {
        Choice::Ready(d) => d.clone(),
        Choice::Boot(d) if !boot => bail!(
            "{} ({}) is shut down and booting is turned off; boot it first",
            d.name,
            d.udid
        ),
        Choice::Boot(d) => {
            on_boot(d);
            let booted = golem_devices::lifecycle::boot_device(d).await?;
            resource_mgr.mark_golem_booted(booted.clone());
            booted
        }
    };
    Ok(Selection { device, bundle })
}

/// The live companion running on `device`, by the UDID it reports.
pub fn companion_on(
    device: &DeviceInfo,
    live: Vec<(u16, CompanionHealth)>,
) -> Option<(u16, CompanionHealth)> {
    live.into_iter().find(|(_, h)| h.device_id == device.udid)
}

/// Every device on both platforms (or on `platform`), in any state.
pub async fn discover_devices(platform: Option<Platform>) -> Vec<DeviceInfo> {
    let mut devices = Vec::new();
    if platform.is_none_or(|p| p == Platform::Ios) {
        devices.extend(
            golem_devices::ios::discover_ios_devices()
                .await
                .unwrap_or_default(),
        );
    }
    if platform.is_none_or(|p| p == Platform::Android) {
        devices.extend(
            golem_devices::android::discover_android_devices()
                .await
                .unwrap_or_default(),
        );
    }
    devices
}

/// The device and app a query names, before any companion work.
#[derive(Debug, Clone)]
pub struct Selection {
    pub device: DeviceInfo,
    pub bundle: String,
}

/// Pick the device and app `query` names. Touches no companion: a caller
/// that leases the device does so between this and [`connect`].
pub async fn select(query: &TargetQuery, apps: &[ProjectAppConfig]) -> Result<Selection> {
    let bundle = resolve_bundle(query, apps)?;
    let devices = discover_devices(query.platform()).await;
    let device = select_device(&devices, query)?.clone();
    Ok(Selection { device, bundle })
}

/// Reuse the selected device's live companion, or start one.
///
/// Starting an iOS companion kills any other companion on that device, so
/// a caller that acts on the device takes its lease first: otherwise it
/// could kill the companion of a run still setting the device up.
pub async fn connect(selection: Selection) -> Result<Target> {
    let Selection { device, bundle } = selection;
    let (port, health) = match companion_on(&device, crate::suite::scan_companions().await) {
        Some(live) => live,
        None => {
            eprintln!("  starting the companion on {} ...", device.name);
            crate::suite::start_companion_for_device(&device).await?
        }
    };
    Ok(Target {
        device,
        bundle,
        port,
        health,
    })
}

/// [`select`] then [`connect`], for a caller that only reads the screen
/// (`golem tree`, `golem probe`) and takes no lease.
pub async fn resolve(query: &TargetQuery, apps: &[ProjectAppConfig]) -> Result<Target> {
    connect(select(query, apps).await?).await
}

/// Every device, with the port of its live companion where one runs.
pub async fn list_devices(platform: Option<Platform>) -> Vec<DeviceEntry> {
    let live = crate::suite::scan_companions().await;
    discover_devices(platform)
        .await
        .into_iter()
        .map(|device| {
            let companion_port = live
                .iter()
                .find(|(_, h)| h.device_id == device.udid)
                .map(|(port, _)| *port);
            DeviceEntry {
                device,
                companion_port,
            }
        })
        .collect()
}

/// One line per device, usable devices first:
/// `ios B9100F0F-… "iPhone 17" os:26.5 booted companion:22087`.
pub fn format_device_entries(entries: &[DeviceEntry]) -> String {
    let mut sorted: Vec<&DeviceEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| !is_usable(&e.device));
    let mut out = String::new();
    for e in sorted {
        let d = &e.device;
        let _ = write!(
            out,
            "{} {} \"{}\" os:{} {}",
            d.platform,
            d.udid,
            d.name,
            d.os_version,
            state_label(d.state)
        );
        if d.physical {
            out.push_str(" physical");
        }
        if let Some(port) = e.companion_port {
            let _ = write!(out, " companion:{port}");
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_devices::DeviceType;

    fn dev(platform: Platform, udid: &str, name: &str, os: &str, state: DeviceState) -> DeviceInfo {
        DeviceInfo {
            name: name.into(),
            udid: udid.into(),
            platform,
            device_type: DeviceType::Phone,
            os_major: os
                .split('.')
                .next()
                .and_then(|m| m.parse().ok())
                .unwrap_or(0),
            os_version: os.into(),
            state,
            physical: state == DeviceState::Connected,
            playstore: false,
            screen_width: None,
            screen_height: None,
            screen_scale: None,
            last_booted: None,
            runtime_id: None,
            device_type_id: None,
        }
    }

    /// Two booted iPhones, one shutdown iPhone that shares a name with a
    /// booted one on another runtime, a booted emulator and a phone.
    fn fleet() -> Vec<DeviceInfo> {
        vec![
            dev(
                Platform::Ios,
                "B9100F0F",
                "iPhone 17",
                "26.5",
                DeviceState::Booted,
            ),
            dev(
                Platform::Ios,
                "A1B2C3D4",
                "iPhone 16",
                "18.6",
                DeviceState::Booted,
            ),
            dev(
                Platform::Ios,
                "E5F6A7B8",
                "iPhone 16",
                "26.5",
                DeviceState::Shutdown,
            ),
            dev(
                Platform::Ios,
                "C0FFEE00",
                "iPad mini",
                "18.6",
                DeviceState::Shutdown,
            ),
            dev(
                Platform::Android,
                "emulator-5554",
                "Pixel_8_Pro",
                "16",
                DeviceState::Booted,
            ),
        ]
    }

    fn session_query(
        os: Option<&str>,
        device_type: Option<&str>,
        device: Option<&str>,
    ) -> TargetQuery {
        TargetQuery {
            os: os.map(|o| OsQuery::parse(o).expect("os")),
            device_type: device_type.map(|t| parse_device_type(t).expect("type")),
            device: device.map(str::to_string),
            ..TargetQuery::default()
        }
    }

    fn chosen(devices: &[DeviceInfo], q: &TargetQuery, busy: &[&str]) -> (bool, String) {
        let in_use = |d: &DeviceInfo| busy.contains(&d.udid.as_str());
        match choose_for_session(devices, q, &in_use).expect("SHALL choose") {
            Choice::Ready(d) => (false, d.udid.clone()),
            Choice::Boot(d) => (true, d.udid.clone()),
        }
    }

    fn refused(devices: &[DeviceInfo], q: &TargetQuery, busy: &[&str]) -> String {
        let in_use = |d: &DeviceInfo| busy.contains(&d.udid.as_str());
        format!(
            "{:#}",
            choose_for_session(devices, q, &in_use).expect_err("SHALL refuse")
        )
    }

    #[test]
    fn os_reads_every_flow_form_for_one_device() {
        let os = |t: &str| OsQuery::parse(t).expect(t);
        assert_eq!(os("ios").version, None);
        assert_eq!(os("android").platform, Platform::Android);
        assert_eq!(
            os("ios:26").version,
            Some(OsVersionSpec::Exact {
                platform: Platform::Ios,
                major: 26
            })
        );
        assert_eq!(
            os("android:34+").version,
            Some(OsVersionSpec::Minimum {
                platform: Platform::Android,
                major: 34
            })
        );
        assert_eq!(
            os("ios:latest").version,
            Some(OsVersionSpec::Latest {
                platform: Platform::Ios,
                count: 1
            })
        );
        for bad in ["web", "ios:", "web:26", "ios:latest:2"] {
            assert!(OsQuery::parse(bad).is_err(), "{bad} SHALL be refused");
        }
        assert_eq!(
            OsQuery::of(Platform::Ios, os("ios:26+").version).text,
            "ios:26+"
        );
    }

    #[test]
    fn a_session_takes_the_free_booted_device_with_the_newest_os() {
        let q = session_query(Some("ios"), None, None);
        assert_eq!(chosen(&fleet(), &q, &[]), (false, "B9100F0F".into()));
        assert_eq!(
            chosen(&fleet(), &q, &["B9100F0F"]),
            (false, "A1B2C3D4".into()),
            "a device a run or a session holds SHALL be skipped"
        );
    }

    #[test]
    fn a_session_boots_the_newest_fitting_device_when_none_runs() {
        let mut devices = fleet();
        devices.retain(|d| d.udid != "B9100F0F");
        assert_eq!(
            chosen(&devices, &session_query(Some("ios:26"), None, None), &[]),
            (true, "E5F6A7B8".into())
        );
        devices[2].device_type = DeviceType::Tablet;
        assert_eq!(
            chosen(
                &devices,
                &session_query(Some("ios"), Some("tablet"), None),
                &[]
            ),
            (true, "C0FFEE00".into())
        );
    }

    #[test]
    fn latest_is_the_newest_os_on_the_host_in_any_state() {
        let mut devices = fleet();
        devices.retain(|d| d.udid != "B9100F0F");
        assert_eq!(
            chosen(
                &devices,
                &session_query(Some("ios:latest"), None, None),
                &[]
            ),
            (true, "E5F6A7B8".into()),
            "a booted iOS 18 SHALL not stand in for latest when iOS 26 exists"
        );
    }

    #[test]
    fn a_named_session_device_is_used_in_any_state() {
        assert_eq!(
            chosen(&fleet(), &session_query(None, None, Some("E5F6A7B8")), &[]),
            (true, "E5F6A7B8".into())
        );
        let err = refused(
            &fleet(),
            &session_query(Some("ios:18"), None, Some("iPhone 17")),
            &[],
        );
        assert!(err.contains("is phone os:26.5, not ios:18"), "{err}");
    }

    #[test]
    fn a_session_prefers_an_emulator_to_a_physical_phone() {
        let mut devices = fleet();
        devices.insert(
            0,
            dev(
                Platform::Android,
                "R5CT",
                "Galaxy",
                "16",
                DeviceState::Connected,
            ),
        );
        assert_eq!(
            chosen(&devices, &session_query(Some("android"), None, None), &[]),
            (false, "emulator-5554".into())
        );
    }

    #[test]
    fn a_session_refuses_when_every_fitting_device_is_busy_or_none_exists() {
        let err = refused(
            &fleet(),
            &session_query(Some("android"), None, None),
            &["emulator-5554"],
        );
        assert!(
            err.contains("in use by a run or a session (Pixel_8_Pro)"),
            "{err}"
        );
        let err = refused(
            &fleet(),
            &session_query(Some("android:30"), None, None),
            &[],
        );
        assert!(
            err.starts_with("no android:30 device on this host"),
            "{err}"
        );
    }

    fn query(platform: Option<Platform>, device: Option<&str>) -> TargetQuery {
        TargetQuery {
            os: platform.map(|p| OsQuery::parse(&p.to_string()).expect("os")),
            device: device.map(str::to_string),
            ..TargetQuery::default()
        }
    }

    fn udid(devices: &[DeviceInfo], q: &TargetQuery) -> String {
        select_device(devices, q)
            .expect("SHALL select")
            .udid
            .clone()
    }

    fn error(devices: &[DeviceInfo], q: &TargetQuery) -> String {
        select_device(devices, q)
            .expect_err("SHALL fail")
            .to_string()
    }

    #[test]
    fn the_only_usable_device_is_selected() {
        let fleet = fleet();
        assert_eq!(
            udid(&fleet, &query(Some(Platform::Android), None)),
            "emulator-5554"
        );
    }

    #[test]
    fn several_usable_devices_without_a_device_is_an_error_listing_them() {
        let fleet = fleet();
        assert_eq!(
            error(&fleet, &query(Some(Platform::Ios), None)),
            "2 booted ios devices; set device to one of:\n  \
             ios A1B2C3D4 \"iPhone 16\" os:18.6\n  \
             ios B9100F0F \"iPhone 17\" os:26.5"
        );
        assert!(error(&fleet, &query(None, None)).starts_with("3 booted devices;"));
    }

    #[test]
    fn a_device_is_named_by_udid_serial_or_name() {
        let fleet = fleet();
        assert_eq!(udid(&fleet, &query(None, Some("b9100f0f"))), "B9100F0F");
        assert_eq!(
            udid(&fleet, &query(None, Some("emulator-5554"))),
            "emulator-5554"
        );
        assert_eq!(udid(&fleet, &query(None, Some("iphone 17"))), "B9100F0F");
        // The shutdown "iPhone 16" on another runtime is no candidate.
        assert_eq!(udid(&fleet, &query(None, Some("iPhone 16"))), "A1B2C3D4");
        assert_eq!(udid(&fleet, &query(None, Some("pixel"))), "emulator-5554");
    }

    #[test]
    fn an_exact_match_beats_a_substring_match() {
        let mut fleet = fleet();
        fleet.push(dev(
            Platform::Ios,
            "D00D",
            "iPhone 17 Pro",
            "26.5",
            DeviceState::Booted,
        ));
        assert_eq!(udid(&fleet, &query(None, Some("iPhone 17"))), "B9100F0F");
    }

    #[test]
    fn a_name_that_matches_several_devices_is_an_error_listing_them() {
        let fleet = fleet();
        assert_eq!(
            error(&fleet, &query(None, Some("iphone"))),
            "2 devices match \"iphone\"; set device to one of:\n  \
             ios A1B2C3D4 \"iPhone 16\" os:18.6\n  \
             ios B9100F0F \"iPhone 17\" os:26.5"
        );
    }

    #[test]
    fn no_match_names_the_booted_devices() {
        let fleet = fleet();
        assert_eq!(
            error(&fleet, &query(Some(Platform::Android), Some("Pixel 9"))),
            "no booted android device matches \"Pixel 9\"; booted devices:\n  \
             android emulator-5554 \"Pixel_8_Pro\" os:16"
        );
        assert_eq!(
            error(&[], &query(None, None)),
            "no booted device; start a simulator or emulator first"
        );
    }

    #[test]
    fn naming_a_device_that_is_not_booted_says_so() {
        let fleet = fleet();
        assert_eq!(
            error(&fleet, &query(None, Some("iPad mini"))),
            "iPad mini (C0FFEE00) is shutdown; boot it first"
        );
    }

    #[test]
    fn the_platform_filter_applies_before_matching() {
        let fleet = fleet();
        assert!(
            error(&fleet, &query(Some(Platform::Android), Some("iPhone 17")))
                .starts_with("no booted android device matches")
        );
    }

    #[test]
    fn a_connected_physical_device_is_usable() {
        let fleet = vec![dev(
            Platform::Ios,
            "00008110",
            "Phone",
            "18.1",
            DeviceState::Connected,
        )];
        assert_eq!(udid(&fleet, &query(None, None)), "00008110");
    }

    fn app(name: &str, bundle: Option<&str>) -> ProjectAppConfig {
        ProjectAppConfig {
            name: name.into(),
            bundle: bundle.map(str::to_string),
            devices: Vec::new(),
            install_script: None,
            install_timeout_ms: None,
            install_env: None,
            profile: None,
        }
    }

    #[test]
    fn the_bundle_comes_from_the_request_then_the_registry() {
        let apps = vec![
            app("app", Some("fail.golem.test")),
            app("b", Some("fail.golem.testb")),
        ];
        let q = |bundle: Option<&str>, app: Option<&str>| TargetQuery {
            bundle: bundle.map(str::to_string),
            app: app.map(str::to_string),
            ..TargetQuery::default()
        };
        assert_eq!(
            resolve_bundle(&q(Some("com.x"), None), &apps).expect("bundle"),
            "com.x"
        );
        assert_eq!(
            resolve_bundle(&q(None, Some("b")), &apps).expect("app"),
            "fail.golem.testb"
        );
        assert_eq!(
            resolve_bundle(&q(None, None), &apps[..1]).expect("only app"),
            "fail.golem.test",
            "the registry's only app SHALL be the default"
        );
        assert_eq!(
            resolve_bundle(&q(None, None), &apps).expect("no default"),
            "",
            "several registered apps SHALL leave the bundle unset"
        );
        assert_eq!(
            resolve_bundle(&q(None, Some("c")), &apps)
                .expect_err("unknown")
                .to_string(),
            "unknown app \"c\"; golem.toml [[apps]] has: app, b"
        );
        assert_eq!(
            resolve_bundle(&q(None, Some("c")), &[])
                .expect_err("no registry")
                .to_string(),
            "unknown app \"c\": golem.toml has no [[apps]]; set bundle instead"
        );
        assert_eq!(
            resolve_bundle(&q(None, Some("x")), &[app("x", None)])
                .expect_err("no bundle")
                .to_string(),
            "app \"x\" in golem.toml has no bundle; set bundle instead"
        );
    }

    fn health(device_id: &str) -> CompanionHealth {
        CompanionHealth {
            platform: "ios".into(),
            version: "0".into(),
            device_name: String::new(),
            os_version: String::new(),
            device_id: device_id.into(),
            max_recording_width: None,
            max_recording_height: None,
        }
    }

    #[test]
    fn the_live_companion_is_matched_by_the_udid_it_reports() {
        let fleet = fleet();
        let live = vec![(22001, health("A1B2C3D4")), (22002, health("B9100F0F"))];
        assert_eq!(companion_on(&fleet[0], live).map(|(p, _)| p), Some(22002));
        assert!(companion_on(&fleet[4], vec![(22001, health("A1B2C3D4"))]).is_none());
    }

    #[test]
    fn the_listing_puts_usable_devices_first_with_their_companion() {
        let fleet = fleet();
        let entries: Vec<DeviceEntry> = fleet
            .into_iter()
            .map(|device| {
                let companion_port = (device.udid == "B9100F0F").then_some(22002);
                DeviceEntry {
                    device,
                    companion_port,
                }
            })
            .collect();
        assert_eq!(
            format_device_entries(&entries),
            "ios B9100F0F \"iPhone 17\" os:26.5 booted companion:22002\n\
             ios A1B2C3D4 \"iPhone 16\" os:18.6 booted\n\
             android emulator-5554 \"Pixel_8_Pro\" os:16 booted\n\
             ios E5F6A7B8 \"iPhone 16\" os:26.5 shutdown\n\
             ios C0FFEE00 \"iPad mini\" os:18.6 shutdown\n"
        );
    }
}
