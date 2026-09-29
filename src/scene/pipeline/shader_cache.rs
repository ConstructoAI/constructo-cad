//! Shader modules shared by every render-pipeline set of a device.
//!
//! A paper-space sheet renders each of its viewports through its own
//! [`super::Pipeline`] (per-slot buffers and caches), and every
//! `Pipeline::new` used to create its own shader modules. On WebGL, wgpu links
//! one GL program per pipeline and caches the programs by the IDENTITY of the
//! shader modules, so each new viewport slot compiled and linked every program
//! again: about 10 s the first time sheet A300 of the AutoCAD set C22-025 was
//! shown, on top of the programs already linked for model space. Handing every
//! `Pipeline` the same modules lets that cache answer; native backends are
//! unaffected (a module is immutable once created).

use std::cell::RefCell;
use std::collections::HashMap;

type Modules = HashMap<(usize, usize), wgpu::ShaderModule>;

thread_local! {
    /// The modules of ONE device: a new device (after a device loss, or a test
    /// creating its own) replaces the whole cache, so no stale device is kept.
    static MODULES: RefCell<Option<(wgpu::Device, Modules)>> = const { RefCell::new(None) };
}

/// The shader module compiled from the WGSL `source` on `device`, created on
/// first use and shared afterwards. `source` is always a `&'static str` built
/// by `include_str!` / `concat!`, so its address identifies it.
pub(crate) fn shared_shader_module(
    device: &wgpu::Device,
    label: &'static str,
    source: &'static str,
) -> wgpu::ShaderModule {
    MODULES.with(|cell| {
        let mut cache = cell.borrow_mut();
        if cache.as_ref().map_or(true, |(cached, _)| cached != device) {
            *cache = Some((device.clone(), Modules::new()));
        }
        let (_, modules) = cache.as_mut().expect("cache set above");
        modules
            .entry((source.as_ptr() as usize, source.len()))
            .or_insert_with(|| {
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(label),
                    source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(source)),
                })
            })
            .clone()
    })
}

/// Number of modules held for the current device (tests).
#[cfg(test)]
pub(crate) fn cached_module_count() -> usize {
    MODULES.with(|cell| cell.borrow().as_ref().map_or(0, |(_, modules)| modules.len()))
}
