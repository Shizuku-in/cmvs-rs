//! Nintendo Switch libnx integration and on-device diagnostics. Button bits are
//! portable; every libnx call is compiled for Horizon only.
#[cfg(target_os = "horizon")]
mod audio;
#[cfg(target_os = "horizon")]
mod font;
#[cfg(target_os = "horizon")]
mod gpu_device;
mod input;
#[cfg(target_os = "horizon")]
mod keyboard;
#[cfg(target_os = "horizon")]
mod performance;
#[cfg(target_os = "horizon")]
mod smoke;
#[cfg(target_os = "horizon")]
mod window;

#[cfg(target_os = "horizon")]
pub use audio::SwitchAudioOut;
#[cfg(target_os = "horizon")]
pub use font::shared_font;
#[cfg(target_os = "horizon")]
pub use gpu_device::SwitchGpu;
pub use input::button;
#[cfg(target_os = "horizon")]
pub use keyboard::software_keyboard;
#[cfg(target_os = "horizon")]
pub use performance::{prefer_core, set_cpu_boost, show_system_error, total_memory};
#[cfg(target_os = "horizon")]
pub use window::SwitchWindow;
