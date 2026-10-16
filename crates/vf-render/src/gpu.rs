//! The one wgpu device of the app. Slint draws the UI with it and the
//! compositor renders preview and export frames with it, so frames are shared
//! as textures without copies.

use vf_core::{Error, Result};

#[derive(Clone)]
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// Picks the fastest adapter (the discrete GPU on laptops).
    pub fn new() -> Result<Self> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        // The UI renderer (Skia) shares this device and only works with the
        // platform's native API: DX12 on Windows, Metal on macOS, Vulkan elsewhere.
        descriptor.backends = native_backend();
        let instance = wgpu::Instance::new(descriptor);
        let request = |fallback: bool| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: fallback,
                compatible_surface: None,
                ..Default::default()
            }))
        };
        // Without a usable GPU (virtual machines, servers), a software one:
        // WARP on Windows, lavapipe (Mesa) on Linux.
        let adapter = request(false).or_else(|_| request(true)).map_err(|error| {
            Error::msg(format!("No graphics adapter is available: {error}. On Linux, install the Vulkan driver for your GPU (or mesa-vulkan-drivers)."))
        })?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ViralFlow"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|error| Error::msg(format!("The graphics device could not be opened: {error}")))?;
        let info = adapter.get_info();
        log::info!("GPU: {} ({:?}, {:?})", info.name, info.backend, info.device_type);
        Ok(Gpu { instance, adapter, device, queue })
    }

    pub fn adapter_name(&self) -> String {
        self.adapter.get_info().name
    }
}

fn native_backend() -> wgpu::Backends {
    if cfg!(target_family = "windows") {
        wgpu::Backends::DX12
    } else if cfg!(target_vendor = "apple") {
        wgpu::Backends::METAL
    } else {
        wgpu::Backends::VULKAN
    }
}
