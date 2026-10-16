//! The GPU side of the window: one device, one swapchain on the picker's
//! surface, and the crcbl UI pass drawing a frame's draw list into it.
//!
//! pikr redraws only when something changed, so each frame is submitted and
//! waited for before the next is recorded; there is never more than one
//! frame in flight to track.
#![allow(unsafe_code)]

use anyhow::{Context, Result, anyhow};
use crcbl_frame::graph::{ImportedImage, InitialClaim, RenderGraph};
use crcbl_frame::{TransientPool, UiRenderer};
use crcbl_hal::{
    CommandEncoderDesc, CompositeAlpha, Device, DeviceDesc, Features, Format, Instance,
    PresentInfo, PresentMode, QueueHandle, QueueKind, ResourceState, SemaphoreSignal,
    SemaphoreWait, SubmitInfo, SurfaceError, SurfaceHandle, SwapchainDesc, SwapchainHandle,
};
use crcbl_shell::SurfaceTarget;
use crcbl_ui::draw_list::DrawList;
use crcbl_ui::image::ImageAtlas;
use crcbl_ui::text::FontAtlas;

/// The GPU instance for this platform: Vulkan on Linux and FreeBSD, D3D12 on
/// Windows (which falls back to the WARP software rasteriser with no GPU),
/// Metal on macOS.
fn open_instance() -> Result<Box<dyn Instance>> {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        crcbl_vk::VkInstance::open()
            .map(|instance| Box::new(instance) as Box<dyn Instance>)
            .map_err(|e| anyhow!("no usable GPU: Vulkan failed to open: {e}"))
    }
    #[cfg(target_os = "windows")]
    {
        crcbl_dx12::Dx12Instance::open()
            .map(|instance| Box::new(instance) as Box<dyn Instance>)
            .ok_or_else(|| anyhow!("no usable GPU: Direct3D 12 failed to open"))
    }
    #[cfg(target_os = "macos")]
    {
        crcbl_mtl::MetalInstance::open()
            .map(|instance| Box::new(instance) as Box<dyn Instance>)
            .ok_or_else(|| anyhow!("no usable GPU: Metal failed to open"))
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "windows",
        target_os = "macos"
    )))]
    {
        Err(anyhow!("pikr has no GPU backend for this platform"))
    }
}

pub struct Gpu {
    // Field order is drop order for nothing here — `destroy` tears these
    // down explicitly, in dependency order.
    instance: Box<dyn Instance>,
    device: Box<dyn Device>,
    queue: QueueHandle,
    surface: SurfaceHandle,
    swapchain: SwapchainHandle,
    desc: SwapchainDesc<'static>,
    renderer: UiRenderer,
    pool: TransientPool,
}

impl Gpu {
    /// Opens a device that can present to `target` at `extent`.
    ///
    /// # Safety contract, enforced by the caller's structure
    ///
    /// The window behind `target` must outlive the returned `Gpu`, and
    /// `destroy` must run before the window is destroyed. `ui::run` owns
    /// both and destroys the `Gpu` first on every exit path.
    pub fn open(target: &SurfaceTarget, extent: (u32, u32), want_alpha: bool) -> Result<Self> {
        let instance = open_instance()?;
        // SAFETY: `target` names the live window `ui::run` created, which it
        // keeps alive until after `Gpu::destroy` has destroyed this surface;
        // `open` runs on the thread that pumps that window.
        let surface = unsafe { instance.create_surface(target) }.context("creating the surface")?;

        // An adapter that cannot present to this surface is skipped, not
        // fatal: the first one that can, wins.
        let mut chosen = None;
        for adapter in instance.adapters() {
            match instance.surface_caps(surface, adapter.id) {
                Ok(caps) if caps.preferred_format().is_some() => {
                    chosen = Some((adapter, caps));
                    break;
                }
                Ok(_) => tracing::debug!(adapter = adapter.name, "no usable surface format"),
                Err(e) => tracing::debug!(adapter = adapter.name, "cannot present: {e}"),
            }
        }
        let Some((adapter, caps)) = chosen else {
            instance.destroy_surface(surface);
            return Err(anyhow!(
                "no usable GPU: no adapter can present to the window"
            ));
        };
        tracing::debug!(adapter = adapter.name, backend = ?instance.backend(), "GPU chosen");

        let device = instance
            .create_device(&DeviceDesc {
                label: Some("pikr"),
                adapter: adapter.id,
                required_features: Features::empty(),
                optional_features: Features::empty(),
                compatible_surface: Some(surface),
            })
            .context("creating the GPU device")?;
        let queue = device
            .queue(QueueKind::Graphics)
            .ok_or_else(|| anyhow!("the GPU device has no graphics queue"))?;

        let format: Format = caps
            .preferred_format()
            .ok_or_else(|| anyhow!("the surface offers no format"))?;
        let transparent = want_alpha
            && caps
                .composite_alpha
                .contains(&CompositeAlpha::PreMultiplied);
        let desc = SwapchainDesc {
            label: Some("pikr"),
            surface,
            format,
            extent,
            image_count: caps
                .min_image_count
                .saturating_add(1)
                .min(caps.max_image_count),
            present_mode: caps.choose_present_mode(&[PresentMode::Fifo]),
            composite_alpha: if transparent {
                CompositeAlpha::PreMultiplied
            } else {
                CompositeAlpha::Opaque
            },
        };
        let swapchain = device
            .create_swapchain(&desc)
            .context("creating the swapchain")?;
        let renderer =
            UiRenderer::new(device.as_ref(), queue, format).context("creating the UI renderer")?;
        Ok(Self {
            instance,
            device,
            queue,
            surface,
            swapchain,
            desc,
            renderer,
            pool: TransientPool::new(),
        })
    }

    /// The image atlas icons are registered in.
    pub fn images_mut(&mut self) -> &mut ImageAtlas {
        self.renderer.images_mut()
    }

    /// Follows a window resize. A zero extent (a minimised window) keeps the
    /// current swapchain.
    pub fn resize(&mut self, extent: (u32, u32)) -> Result<()> {
        if extent.0 == 0 || extent.1 == 0 || extent == self.desc.extent {
            return Ok(());
        }
        self.desc.extent = extent;
        self.reconfigure()
    }

    fn reconfigure(&mut self) -> Result<()> {
        self.device.wait_idle().context("waiting for the GPU")?;
        self.device
            .reconfigure_swapchain(self.swapchain, &self.desc)
            .context("reconfiguring the swapchain")
    }

    /// Draws `list` and presents it. `false` when the swapchain was out of
    /// date and was reconfigured instead, so the caller draws again.
    pub fn present(&mut self, list: &DrawList, atlas: &FontAtlas) -> Result<bool> {
        let acquired = match self.device.acquire_next_frame(self.swapchain) {
            Ok(frame) => frame,
            Err(SurfaceError::OutOfDate) => {
                self.reconfigure()?;
                return Ok(false);
            }
            Err(e) => return Err(e).context("acquiring a frame"),
        };
        self.renderer
            .begin_frame(self.device.as_ref(), list, atlas, 1.0)
            .context("preparing the UI pass")?;

        let compiled = {
            let mut graph = RenderGraph::new(self.queue);
            // An acquired image's contents are undefined, and it goes back to
            // the window system for presenting.
            let target = graph.import_image(
                "swapchain",
                ImportedImage {
                    image: acquired.image,
                    view: acquired.view,
                    format: self.desc.format,
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

        let mut encoder = self.device.create_command_encoder(&CommandEncoderDesc {
            label: Some("pikr frame"),
            queue: self.queue,
        });
        compiled
            .execute(self.device.as_ref(), &mut self.pool, encoder.as_mut(), None)
            .context("recording the frame")?;
        let commands = encoder.finish().context("finishing the frame")?;

        let waits: Vec<SemaphoreWait> = acquired
            .acquire_semaphore
            .map(|semaphore| SemaphoreWait {
                semaphore,
                value: 0,
            })
            .into_iter()
            .collect();
        let signals: Vec<SemaphoreSignal> = acquired
            .present_semaphore
            .map(|semaphore| SemaphoreSignal {
                semaphore,
                value: 0,
            })
            .into_iter()
            .collect();
        self.device
            .submit(
                self.queue,
                &SubmitInfo {
                    command_buffers: &[commands],
                    waits: &waits,
                    signals: &signals,
                },
            )
            .context("submitting the frame")?;
        let presented = self.device.present(
            self.queue,
            &PresentInfo {
                swapchain: self.swapchain,
                waits: acquired.present_semaphore.as_slice(),
                present_id: None,
            },
        );
        // One frame in flight: wait for it, then free what it used.
        self.device.wait_idle().context("waiting for the frame")?;
        self.device.destroy_command_buffer(commands);
        self.pool.retire_unused(self.device.as_ref());
        match presented {
            Ok(()) if acquired.suboptimal => {
                self.reconfigure()?;
                Ok(true)
            }
            Ok(()) => Ok(true),
            Err(SurfaceError::OutOfDate) => {
                self.reconfigure()?;
                Ok(false)
            }
            Err(e) => Err(e).context("presenting the frame"),
        }
    }

    /// Releases everything, in dependency order. Must run before the window
    /// is destroyed.
    pub fn destroy(mut self) {
        if let Err(e) = self.device.wait_idle() {
            tracing::warn!("waiting for the GPU at exit: {e}");
        }
        self.renderer.destroy(self.device.as_ref());
        self.pool.destroy(self.device.as_ref());
        self.device.destroy_swapchain(self.swapchain);
        drop(self.device);
        self.instance.destroy_surface(self.surface);
    }
}
