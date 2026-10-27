//! The GPU pikr last opened on, kept in the state directory so the next
//! launch opens that one again: on Vulkan through its driver alone, so the
//! loader no longer loads and probes every installed driver (a hybrid
//! laptop's dGPU among them) on each start. crcbl-app matches the saved
//! choice to an adapter, checks it still holds and falls back when it does
//! not; pikr only keeps the value. See `crcbl_app::gpu_choice`.

use std::path::Path;

use crcbl_app::backend::GpuBackend;
use crcbl_app::gpu_choice::{ChoiceOutcome, GpuChoice};

/// The state file, in crcbl's own text form for a choice.
const FILE: &str = "gpu-choice";

/// The choice the last launch saved. A missing, unreadable or malformed file
/// is no choice: the open then chooses as it would with none, and its choice
/// is saved over the file.
pub fn load() -> Option<GpuChoice> {
    crate::picker::state_file_path(FILE).and_then(|path| load_from(&path))
}

fn load_from(path: &Path) -> Option<GpuChoice> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "cannot read the saved GPU choice");
            return None;
        }
    };
    text.parse()
        .inspect_err(|e| {
            tracing::warn!(error = %e, path = %path.display(), "ignoring the saved GPU choice");
        })
        .ok()
}

/// What to save after an open whose saved choice came to `outcome`: the
/// open's own `choice` when none was saved or the saved one went stale, on a
/// backend that opens by one, and when nobody chose for this run
/// (`Presenter::gpu_choice_overridden`). Nothing when the saved one was used
/// or ignored.
pub fn to_save(outcome: &ChoiceOutcome, choice: &GpuChoice, overridden: bool) -> Option<GpuChoice> {
    let fresh = matches!(outcome, ChoiceOutcome::NotGiven | ChoiceOutcome::Stale(_));
    let opens_by_one = matches!(choice.backend(), GpuBackend::Vulkan | GpuBackend::Dx12);
    (fresh && opens_by_one && !overridden).then(|| choice.clone())
}

/// Saves `choice` for the next launch, first finding its Vulkan driver.
/// Slow on Vulkan: finding the driver loads every installed one, the cost the
/// saved choice then spares each later launch, so pikr runs this once its
/// window is gone. Best-effort; warnings via `tracing` on failure.
pub fn save(mut choice: GpuChoice) {
    if choice.backend() == GpuBackend::Vulkan && !choice.locate_vulkan_driver() {
        tracing::debug!("no Vulkan driver alone shows the adapter; saving it without one");
    }
    if let Some(path) = crate::picker::state_file_path(FILE) {
        save_to(&path, &choice);
    }
}

fn save_to(path: &Path, choice: &GpuChoice) {
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(error = %e, path = %parent.display(), "create state dir");
        return;
    }
    match crate::picker::write_private_state(path, choice.to_string().as_bytes()) {
        Ok(()) => tracing::debug!(path = %path.display(), "GPU choice saved"),
        Err(e) => tracing::warn!(error = %e, path = %path.display(), "save the GPU choice"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crcbl_app::gpu_choice::Ignored;
    use crcbl_hal::{AdapterId, AdapterInfo, BackendKind, DeviceType};

    fn choice(backend: GpuBackend) -> GpuChoice {
        GpuChoice::of(
            backend,
            &AdapterInfo {
                id: AdapterId(0),
                name: "Intel(R) Graphics (RPL-U)".to_owned(),
                vendor_id: 0x8086,
                device_id: 0xa721,
                device_type: DeviceType::Integrated,
                driver: "Intel open-source Mesa driver".to_owned(),
                backend: BackendKind::Vulkan,
                luid: None,
            },
        )
    }

    #[test]
    fn a_new_or_stale_choice_is_saved() {
        let vk = choice(GpuBackend::Vulkan);
        assert_eq!(
            to_save(&ChoiceOutcome::NotGiven, &vk, false),
            Some(vk.clone())
        );
        let stale = ChoiceOutcome::Stale("gone".into());
        assert_eq!(to_save(&stale, &vk, false), Some(vk.clone()));
        let dx12 = choice(GpuBackend::Dx12);
        assert_eq!(to_save(&ChoiceOutcome::NotGiven, &dx12, false), Some(dx12));
    }

    #[test]
    fn a_used_or_ignored_choice_is_kept_as_saved() {
        let vk = choice(GpuBackend::Vulkan);
        assert_eq!(to_save(&ChoiceOutcome::Used, &vk, false), None);
        let ignored = ChoiceOutcome::Ignored(Ignored::Overridden("CRCBL_ADAPTER"));
        assert_eq!(to_save(&ignored, &vk, false), None);
    }

    /// A pinned run with nothing saved must not save its pin as everyone's
    /// choice.
    #[test]
    fn a_run_someone_chose_for_saves_nothing() {
        let vk = choice(GpuBackend::Vulkan);
        assert_eq!(to_save(&ChoiceOutcome::NotGiven, &vk, true), None);
    }

    /// Metal and the test backend do not open by a choice, so there is
    /// nothing worth saving.
    #[test]
    fn a_backend_without_choices_saves_nothing() {
        for backend in [GpuBackend::Metal, GpuBackend::Null] {
            assert_eq!(
                to_save(&ChoiceOutcome::NotGiven, &choice(backend), false),
                None
            );
        }
    }

    #[test]
    fn a_saved_choice_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pikr").join(FILE);
        let vk = choice(GpuBackend::Vulkan)
            .with_vulkan_driver("/usr/share/vulkan/icd.d/intel_icd.x86_64.json".into());
        save_to(&path, &vk);
        assert_eq!(load_from(&path), Some(vk));
    }

    #[test]
    fn a_missing_or_malformed_file_is_no_choice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        assert_eq!(load_from(&path), None);
        std::fs::write(&path, "not a choice\n").unwrap();
        assert_eq!(load_from(&path), None);
    }
}
