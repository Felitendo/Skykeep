//! Whether this computer runs on its battery, so the checks for changes made elsewhere can
//! come less often while it does.
//!
//! Linux: UPower's `OnBattery` on the system bus, read once and then followed as it changes.
//! Elsewhere, or with no UPower, it reads as plugged in and nothing changes. Only the polls
//! that stand in for a push slow down, and never while a window has the focus: the person
//! looking at it sees changes as fast as ever, and Apple's own pushes are never held back.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static ON_BATTERY: AtomicBool = AtomicBool::new(false);

pub fn on_battery() -> bool {
    ON_BATTERY.load(Ordering::Relaxed)
}

/// `every` as it is on mains power, or twice as long on the battery — unless a window has the
/// focus, when it stays as it is.
pub fn pace(every: Duration, focused: bool) -> Duration {
    stretch(every, focused, on_battery())
}

fn stretch(every: Duration, focused: bool, battery: bool) -> Duration {
    if battery && !focused {
        every * 2
    } else {
        every
    }
}

/// Follow UPower for as long as the app runs. Once per run.
pub fn watch() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(follow);
}

fn follow() {
    #[cfg(target_os = "linux")]
    tauri::async_runtime::spawn(async {
        use futures_util::StreamExt;
        let Ok(bus) = zbus::Connection::system().await else {
            return;
        };
        let Ok(builder) = zbus::proxy::Builder::<zbus::Proxy<'static>>::new(&bus)
            .destination("org.freedesktop.UPower")
            .and_then(|b| b.path("/org/freedesktop/UPower"))
            .and_then(|b| b.interface("org.freedesktop.UPower"))
        else {
            return;
        };
        let Ok(upower) = builder.build().await else {
            return;
        };
        if let Ok(now) = upower.get_property::<bool>("OnBattery").await {
            ON_BATTERY.store(now, Ordering::Relaxed);
        }
        let mut changes = upower.receive_property_changed::<bool>("OnBattery").await;
        while let Some(change) = changes.next().await {
            if let Ok(now) = change.get().await {
                ON_BATTERY.store(now, Ordering::Relaxed);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_battery_slows_only_what_nobody_is_looking_at() {
        let every = Duration::from_secs(15);
        assert_eq!(stretch(every, false, false), every);
        assert_eq!(stretch(every, false, true), Duration::from_secs(30));
        assert_eq!(
            stretch(every, true, true),
            every,
            "a focused window keeps its pace"
        );
    }
}
