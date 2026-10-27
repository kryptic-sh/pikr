//! The GPU side of the window: crcbl-app's [`Presenter`] (device, swapchain,
//! adapter choice and frame pacing) and the crcbl UI pass drawing a frame's
//! draw list into it.
//!
//! pikr redraws only when something changed, so it keeps one frame in
//! flight: each frame has finished on the GPU before the next is recorded.

use anyhow::{Context, Result, anyhow};
use crcbl_app::adapter::{PowerPreference, pin};
use crcbl_app::backend::GpuBackend;
use crcbl_app::gpu_choice::{ChoiceOutcome, GpuChoice};
use crcbl_app::present::{
    FrameOutcome, Pacing, PresentError, Presenter, PresenterDesc, Transparency,
};
use crcbl_frame::graph::{ImportedImage, InitialClaim, RenderGraph};
use crcbl_frame::{TransientPool, UiRenderer};
use crcbl_hal::{CommandEncoderDesc, CompositeAlpha, Features, ResourceState};
use crcbl_shell::{Shell, ShellEvent, WindowId};
use crcbl_ui::draw_list::DrawList;
use crcbl_ui::image::ImageAtlas;
use crcbl_ui::text::FontAtlas;

use crate::ui::gpu_choice;

/// The GPU backend for this platform: Vulkan on Linux and FreeBSD, D3D12 on
/// Windows (which falls back to the WARP software rasteriser with no GPU, and
/// is the backend that draws a transparent Win32 window), Metal on macOS.
/// Named rather than left to crcbl-app's table, which on Windows would open
/// Vulkan first and only then fall through to D3D12.
pub fn platform_backend() -> Result<GpuBackend> {
    if cfg!(any(target_os = "linux", target_os = "freebsd")) {
        Ok(GpuBackend::Vulkan)
    } else if cfg!(target_os = "windows") {
        Ok(GpuBackend::Dx12)
    } else if cfg!(target_os = "macos") {
        Ok(GpuBackend::Metal)
    } else {
        Err(anyhow!("pikr has no GPU backend for this platform"))
    }
}

pub struct Gpu {
    presenter: Presenter,
    renderer: UiRenderer,
    pool: TransientPool,
    /// The choice to save for the next launch; see [`Gpu::choice_to_save`].
    choice_to_save: Option<GpuChoice>,
}

impl Gpu {
    /// Opens a device on `backend` that presents to `window` at `extent`.
    ///
    /// The adapter is crcbl's rule: `CRCBL_ADAPTER` if set, else the GPU
    /// driving the window's display when it is discrete, else integrated
    /// before discrete, so a launcher lets a laptop's dGPU sleep; an adapter
    /// that cannot present the window is stepped past. A transparent window
    /// no adapter can composite is drawn opaque, with a warning.
    ///
    /// `saved` is the choice an earlier launch saved, opened again where it
    /// still holds and nobody chose for this run; see [`crate::ui::gpu_choice`].
    ///
    /// `window` must outlive the returned `Gpu`, and [`Gpu::destroy`] must
    /// run before it is destroyed: `ui::run` owns both and destroys the
    /// `Gpu` first on every exit path.
    pub fn open(
        shell: &dyn Shell,
        window: WindowId,
        extent: (u32, u32),
        backend: GpuBackend,
        saved: Option<&GpuChoice>,
    ) -> Result<Self> {
        let pinned = pin();
        let desc = PresenterDesc {
            label: "pikr",
            backend: Some(backend),
            adapter_pin: pinned.as_deref(),
            power_preference: PowerPreference::LowPower,
            transparency: Transparency::Preferred,
            // No present feedback or timing: the frame request already holds
            // presents back until the window system asks, and pikr has no
            // frame rate to settle.
            optional_features: Features::empty(),
            pacing: Pacing::Vsync,
            frames_in_flight: 1,
            gpu_choice: saved,
            ..PresenterDesc::default()
        };
        let presenter = Presenter::open(shell, window, extent, &desc).map_err(no_gpu)?;
        let transparent = shell
            .surface_target(window)
            .is_ok_and(|target| target.transparent());
        if transparent && presenter.composite_alpha() == CompositeAlpha::Opaque {
            tracing::warn!(
                adapter = presenter.adapter().map(|a| a.name),
                "the GPU surface cannot draw pikr transparent; drawing it opaque, so the corners outside the rounded panel show black"
            );
        }
        tracing::debug!(
            adapter = presenter.adapter().map(|a| a.name),
            ?backend,
            saved_choice = ?presenter.gpu_choice_outcome(),
            "GPU chosen"
        );
        if let ChoiceOutcome::Stale(why) = presenter.gpu_choice_outcome() {
            tracing::info!("the saved GPU choice no longer holds ({why}); saving a new one");
        }
        let choice_to_save = gpu_choice::to_save(
            presenter.gpu_choice_outcome(),
            presenter.gpu_choice(),
            presenter.gpu_choice_overridden().is_some(),
        );
        let renderer = UiRenderer::new(presenter.device(), presenter.queue(), presenter.format())
            .context("creating the UI renderer")?;
        Ok(Self {
            presenter,
            renderer,
            pool: TransientPool::new(),
            choice_to_save,
        })
    }

    /// The GPU choice for the next launch to open on, when this launch had
    /// none or its saved one went stale; `None` when there is nothing new to
    /// save. Saving it ([`gpu_choice::save`]) is slow, so it waits until the
    /// window is gone.
    pub fn choice_to_save(&self) -> Option<GpuChoice> {
        self.choice_to_save.clone()
    }

    /// The image atlas icons are registered in.
    pub fn images_mut(&mut self) -> &mut ImageAtlas {
        self.renderer.images_mut()
    }

    /// Follows a window resize. A zero extent (a minimised window) keeps the
    /// current swapchain.
    pub fn resize(&mut self, extent: (u32, u32)) -> Result<()> {
        self.presenter
            .resize(extent)
            .context("reconfiguring the swapchain")
    }

    /// Takes what `event` says about the frame request: its answer lets the
    /// next frame be presented.
    pub fn observe(&mut self, event: &ShellEvent) {
        self.presenter.observe(event);
    }

    /// Whether the window system has asked for the next frame, or none was
    /// presented yet. A FIFO present before then can block the loop, keys
    /// and all, for as long as the window is not shown (an output switched
    /// off).
    pub fn frame_due(&self) -> bool {
        self.presenter.frame_due()
    }

    /// Draws `list` and presents it, asking `shell` for the next frame.
    /// `false` when the swapchain was out of date and was reconfigured
    /// instead, so the caller draws again.
    pub fn present(
        &mut self,
        shell: &mut dyn Shell,
        list: &DrawList,
        atlas: &FontAtlas,
    ) -> Result<bool> {
        let Some(acquired) = self.presenter.acquire().context("acquiring a frame")? else {
            return Ok(false);
        };
        let device = self.presenter.device();
        self.renderer
            .begin_frame(device, list, atlas, 1.0)
            .context("preparing the UI pass")?;

        let compiled = {
            let mut graph = RenderGraph::new(self.presenter.queue());
            // An acquired image's contents are undefined, and it goes back to
            // the window system for presenting.
            let target = graph.import_image(
                "swapchain",
                ImportedImage {
                    image: acquired.image,
                    view: acquired.view,
                    format: self.presenter.format(),
                    extent: acquired.extent,
                    initial: ResourceState::Undefined,
                    claim: InitialClaim::Acquired,
                    final_state: ResourceState::Present,
                },
            );
            // The UI passes load what is there, so something must clear first.
            // Clear to transparent: where the panel draws nothing (its rounded
            // corners), the desktop shows through a premultiplied swapchain.
            graph
                .add_render_pass("clear")
                .clear_color(target, [0.0; 4])
                .execute(|_| {});
            self.renderer
                .add_passes(&mut graph, target, acquired.extent);
            graph.compile(&self.pool).context("compiling the frame")?
        };

        let mut encoder = device.create_command_encoder(&CommandEncoderDesc {
            label: Some("pikr frame"),
            queue: self.presenter.queue(),
        });
        compiled
            .execute(device, &mut self.pool, encoder.as_mut(), None)
            .context("recording the frame")?;
        let commands = encoder.finish().context("finishing the frame")?;

        // Rides this present's commit; its answer lets the next one go.
        self.presenter.request_frame(shell)?;
        // With one frame in flight this returns after the frame has finished
        // on the GPU, so what it used can be reused or freed.
        let outcome = self
            .presenter
            .submit_and_present(&acquired, commands)
            .context("presenting the frame")?;
        self.pool.retire_unused(self.presenter.device());
        Ok(outcome == FrameOutcome::Presented)
    }

    /// Releases everything, in dependency order. Must run before the window
    /// is destroyed.
    pub fn destroy(mut self) {
        if let Err(e) = self.presenter.drain() {
            tracing::warn!("waiting for the GPU at exit: {e}");
        }
        self.renderer.destroy(self.presenter.device());
        self.pool.destroy(self.presenter.device());
        if let Err(e) = self.presenter.destroy() {
            tracing::warn!("releasing the GPU at exit: {e}");
        }
    }
}

/// A failed open as pikr reports it. crcbl-app's own wording for a backend
/// that would not open suggests its `--backend null` flag, which pikr does
/// not have.
fn no_gpu(error: PresentError) -> anyhow::Error {
    match error {
        PresentError::NoBackend(e) => anyhow!("no usable GPU: {e}"),
        e => anyhow!("no usable GPU: {e}"),
    }
}
