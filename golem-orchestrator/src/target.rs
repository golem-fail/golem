//! Device and app selection for the interactive commands: `golem tree`,
//! `golem do`, `golem probe`, sessions and the MCP `devices` tool.
//!
//! A suite run picks devices from flow constraints. An interactive command
//! names one device and one app, and acts on exactly that pair: when the
//! request matches more than one device it fails with the candidates
//! instead of picking one.

use std::fmt::Write;

use anyhow::{bail, Result};
use golem_devices::{DeviceInfo, DeviceState, Platform};
use golem_driver::CompanionHealth;
use golem_parser::ProjectAppConfig;

/// What the caller asked for. Every field is optional.
#[derive(Debug, Clone, Default)]
pub struct TargetQuery {
    pub platform: Option<Platform>,
    /// UDID, serial or name.
    pub device: Option<String>,
    /// Bundle id, used as given.
    pub bundle: Option<String>,
    /// An app name from the project `[[apps]]` registry.
    pub app: Option<String>,
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
    let on_platform: Vec<&DeviceInfo> = devices
        .iter()
        .filter(|d| query.platform.is_none_or(|p| d.platform == p))
        .collect();
    let usable: Vec<&DeviceInfo> = on_platform
        .iter()
        .copied()
        .filter(|d| is_usable(d))
        .collect();
    let platform = query.platform.map_or_else(String::new, |p| format!("{p} "));

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

    let wanted_lc = wanted.to_lowercase();
    let steps: [&dyn Fn(&DeviceInfo) -> bool; 3] = [
        &|d| d.udid.to_lowercase() == wanted_lc,
        &|d| d.name.to_lowercase() == wanted_lc,
        &|d| {
            d.udid.to_lowercase().contains(&wanted_lc) || d.name.to_lowercase().contains(&wanted_lc)
        },
    ];
    for matches_step in steps {
        let hits: Vec<&DeviceInfo> = usable.iter().copied().filter(|d| matches_step(d)).collect();
        match hits.as_slice() {
            [] => continue,
            [only] => return Ok(only),
            many => {
                return Err(ambiguous(
                    &format!("{} devices match \"{wanted}\"", many.len()),
                    many,
                ))
            }
        }
    }

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
    let devices = discover_devices(query.platform).await;
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

    fn query(platform: Option<Platform>, device: Option<&str>) -> TargetQuery {
        TargetQuery {
            platform,
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
