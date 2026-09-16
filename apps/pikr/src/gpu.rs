//! Which wgpu backends the picker asks floem for.
//!
//! Only compiled where a caller exists — see the `mod gpu` declaration.
//!
//! floem lets wgpu enable every backend, so Vulkan is enumerated at startup.
//! Enumeration reaches every DRM device, and touching a runtime-suspended GPU
//! resumes it. On a hybrid laptop that is the discrete card: the resume costs
//! over a second before the first frame, on a GPU that then renders nothing,
//! because the layer surface belongs to the card driving the display.
//!
//! Restricting the request to GL keeps enumeration off it, but GL is not safe
//! to ask for everywhere, and the failure is fatal: when no adapter can serve
//! the surface floem panics acquiring one instead of falling back. Observed on
//! an RTX 2080 Ti with NVIDIA's proprietary driver — wgpu does enumerate a GL
//! adapter there, and then rejects it with "not compatible with surface:
//! Failed to retrieve surface capabilities", which reaches floem as
//! `AdapterNotFoundError`.
//!
//! So GL is asked for only on positive evidence: every card readable, one of
//! them parked, and one awake on a driver in [`GL_SURFACE_DRIVERS`]. An
//! unrecognised driver keeps floem's default, which costs only the wake this
//! module exists to avoid — the allow-list is the conservative direction.
//!
//! Two limits worth knowing. The state is sampled once at startup, so a card
//! that parks or wakes later in the session is not reflected. And a kernel
//! driver name only stands in for what the EGL stack can do: a machine with a
//! parked card, an allow-listed driver, and no working EGL (a container
//! without `libwayland-egl`) is still asked for GL. `WGPU_BACKEND` overrides
//! the choice either way.

use std::path::{Path, PathBuf};

/// Where the kernel exposes DRM devices.
const DRM_ROOT: &str = "/sys/class/drm";

/// Drivers whose GL stack is known to present to a Wayland layer surface, as
/// the basename of a card's `device/driver` link. Mesa drivers, which share
/// the EGL path this was verified on (`i915`, on a hybrid laptop).
///
/// It is an allow-list because the name is a bus driver, not a promise: the
/// sysfb family (`simple-framebuffer`, `bochs-drm`) and drivers with no GL
/// userspace at all (`nova-core`, for NVIDIA GSP cards) would otherwise read
/// as capable. Excluding a driver that would have worked costs one slow start;
/// including one that does not costs a panic.
const GL_SURFACE_DRIVERS: &[&str] = &["amdgpu", "i915", "nouveau", "radeon", "xe"];

/// The backends to request, or `None` to accept floem's default.
pub fn preferred_backends() -> Option<floem::wgpu::Backends> {
    backends_for(Path::new(DRM_ROOT))
}

/// A DRM card's driver and runtime-power state.
struct Card {
    driver: String,
    status: String,
}

/// The rule, split from the sysfs path so tests can supply a tree.
fn backends_for(drm_root: &Path) -> Option<floem::wgpu::Backends> {
    let cards = read_cards(drm_root)?;
    // Only the exact string "suspended" is parked; everything else the kernel
    // can write ("active", "unsupported", "error", the transitional
    // "suspending"/"resuming") reads as awake. That makes a parked card harder
    // to find, which withholds GL; it also makes an awake card easier to find,
    // which is why the driver allow-list, not the power state, is what admits
    // one.
    let any_parked = cards.iter().any(|card| card.status == "suspended");
    let awake_card_with_gl = cards.iter().any(|card| {
        card.status != "suspended" && GL_SURFACE_DRIVERS.contains(&card.driver.as_str())
    });
    (any_parked && awake_card_with_gl).then_some(floem::wgpu::Backends::GL)
}

/// Every `cardN` under `drm_root`, or `None` if the set cannot be read whole.
///
/// Connectors (`card0-DP-1`) and render nodes (`renderD128`) carry a `device/`
/// symlink too, but a render node's resolves to the same PCI device as its own
/// card and a connector's to the DRM minor, which reports `unsupported`.
/// Reading cards alone pairs one driver with one power state and counts each
/// device once.
///
/// A `cardN` whose files cannot be read abandons the whole answer rather than
/// being skipped: skipping would hide it from the driver check, and the two
/// mistakes do not cost the same — guessing the default loses a faster start,
/// guessing GL where it cannot present panics. An entry whose own directory
/// listing errors, or whose name is not UTF-8, is still skipped; neither is
/// reachable on sysfs, so it is not worth failing the whole probe over.
fn read_cards(drm_root: &Path) -> Option<Vec<Card>> {
    let mut cards = Vec::new();
    for entry in std::fs::read_dir(drm_root).ok()?.filter_map(Result::ok) {
        if entry.file_name().to_str().is_some_and(is_card_dir) {
            cards.push(read_card(&entry.path())?);
        }
    }
    Some(cards)
}

/// Reading `runtime_status` reports the state the kernel already holds; unlike
/// a connector's `status` it starts no probe, so it does not resume the device
/// this code exists to leave alone. Observed: repeated reads of a suspended
/// card left both its status and its `runtime_active_time` unchanged.
fn read_card(card: &Path) -> Option<Card> {
    let status = std::fs::read_to_string(card.join("device/power/runtime_status")).ok()?;
    let driver: PathBuf = std::fs::read_link(card.join("device/driver")).ok()?;
    Some(Card {
        driver: driver.file_name()?.to_str()?.to_owned(),
        status: status.trim().to_owned(),
    })
}

/// `card0` yes, `card0-DP-1` and `renderD128` no — see [`read_cards`].
fn is_card_dir(name: &str) -> bool {
    name.strip_prefix("card")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::{backends_for, is_card_dir};
    use std::path::{Path, PathBuf};

    /// A DRM root, plus the tempdir owning it — hold the tempdir for the test's
    /// lifetime or the tree is deleted underneath it.
    ///
    /// The drivers the cards link to live beside the root, not inside it, so
    /// the root holds cards and nothing else. That matters: with them inside,
    /// removing the `is_card_dir` filter made `read_cards` stumble over the
    /// `drivers/` directory and fail closed, which is the same answer the
    /// filter gives, so no test could see the filter disappear.
    fn tree() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let drm = dir.path().join("drm");
        std::fs::create_dir_all(&drm).unwrap();
        (dir, drm)
    }

    /// Build a `cardN` with the given driver and runtime-power state.
    fn card(drm: &Path, name: &str, driver: &str, status: &str) {
        let device = drm.join(name).join("device");
        std::fs::create_dir_all(device.join("power")).unwrap();
        std::fs::write(device.join("power/runtime_status"), format!("{status}\n")).unwrap();
        let driver_dir = drm.parent().unwrap().join("drivers").join(driver);
        std::fs::create_dir_all(&driver_dir).unwrap();
        std::os::unix::fs::symlink(&driver_dir, device.join("driver")).unwrap();
    }

    #[test]
    fn hybrid_laptop_asks_for_gl() {
        let (_dir, root) = tree();
        card(&root, "card0", "nouveau", "suspended");
        card(&root, "card1", "i915", "active");
        assert_eq!(
            backends_for(&root),
            Some(floem::wgpu::Backends::GL),
            "a parked card beside an awake Mesa card is the case GL exists for"
        );
    }

    #[test]
    fn parked_nvidia_beside_awake_igpu_asks_for_gl() {
        let (_dir, root) = tree();
        // The configuration the module exists for. A rule that rejected any
        // NVIDIA card anywhere, rather than an awake one, would answer None
        // here and pass every other test in this file.
        card(&root, "card0", "nvidia", "suspended");
        card(&root, "card1", "i915", "active");
        assert_eq!(backends_for(&root), Some(floem::wgpu::Backends::GL));
    }

    #[test]
    fn transitional_and_unsupported_statuses_read_as_awake() {
        let (_dir, root) = tree();
        // Only the exact string parks a card; a machine with runtime PM off
        // reports "unsupported" everywhere and must not look parked.
        card(&root, "card0", "i915", "unsupported");
        card(&root, "card1", "amdgpu", "suspending");
        assert_eq!(backends_for(&root), None);
    }

    #[test]
    fn lone_parked_nvidia_keeps_the_default() {
        let (_dir, root) = tree();
        // Nothing awake to render on: the parked card is the only one, and
        // its stack is the one without a GL adapter.
        card(&root, "card0", "nvidia", "suspended");
        assert_eq!(backends_for(&root), None);
    }

    #[test]
    fn parked_card_beside_awake_nvidia_keeps_the_default() {
        let (_dir, root) = tree();
        card(&root, "card0", "i915", "suspended");
        card(&root, "card1", "nvidia", "active");
        assert_eq!(
            backends_for(&root),
            None,
            "NVIDIA's GL adapter cannot serve the layer surface, so it panics"
        );
    }

    #[test]
    fn unrecognised_awake_driver_keeps_the_default() {
        let (_dir, root) = tree();
        card(&root, "card0", "nouveau", "suspended");
        // A framebuffer or GSP card: not NVIDIA's driver, but no reason to
        // believe its GL can present either.
        card(&root, "card1", "simple-framebuffer", "active");
        assert_eq!(backends_for(&root), None);
    }

    #[test]
    fn one_unreadable_card_abandons_an_otherwise_gl_answer() {
        let (_dir, root) = tree();
        // Without the third card this is the hybrid-laptop shape, so the rule
        // would answer GL; the unreadable card is the only thing withholding
        // it, which is what makes this fail if the card were skipped instead.
        card(&root, "card0", "nouveau", "suspended");
        card(&root, "card1", "i915", "active");
        // Present but driverless: it could be the NVIDIA one.
        std::fs::create_dir_all(root.join("card2/device/power")).unwrap();
        std::fs::write(root.join("card2/device/power/runtime_status"), "active\n").unwrap();
        assert_eq!(backends_for(&root), None);
    }

    #[test]
    fn single_awake_card_keeps_the_default() {
        let (_dir, root) = tree();
        // Allow-listed on purpose: with nothing parked there is no wake to
        // avoid, and only the parked requirement can be holding GL back.
        card(&root, "card1", "i915", "active");
        assert_eq!(backends_for(&root), None);
    }

    #[test]
    fn nothing_parked_keeps_the_default() {
        let (_dir, root) = tree();
        card(&root, "card0", "nouveau", "active");
        card(&root, "card1", "i915", "active");
        assert_eq!(backends_for(&root), None);
    }

    #[test]
    fn missing_drm_tree_keeps_the_default() {
        let (_dir, root) = tree();
        assert_eq!(backends_for(&root.join("absent")), None);
    }

    #[test]
    fn render_nodes_and_connectors_are_not_counted_as_cards() {
        let (_dir, root) = tree();
        // Synthetic: a render node mirrors its own card's power state, so on
        // real hardware these would agree. The filter is what keeps one device
        // from being counted twice, and this pins that it still filters.
        card(&root, "card1", "i915", "active");
        card(&root, "renderD129", "i915", "suspended");
        card(&root, "card1-DP-1", "i915", "suspended");
        assert_eq!(backends_for(&root), None);
    }

    #[test]
    fn card_dirs_only() {
        assert!(is_card_dir("card0"));
        assert!(is_card_dir("card12"));
        assert!(!is_card_dir("card0-DP-1"));
        assert!(!is_card_dir("card1-eDP-1"));
        assert!(!is_card_dir("renderD128"));
        assert!(!is_card_dir("card"));
        assert!(!is_card_dir("version"));
    }
}
