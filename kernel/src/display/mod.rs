//! Linear Framebuffer & 2D Desktop Compositor for KntonOS.

pub mod color;
pub mod desktop;
pub mod draw;
pub mod font;
pub mod framebuffer;
pub mod vrr_pacer;
pub mod dock_manager;

pub use color::Color;
pub use desktop::{DesktopShell, WindowRect, TASKBAR_HEIGHT};
pub use draw::{draw_char, draw_rect, draw_rect_outline, draw_string};
pub use font::{FONT_HEIGHT, FONT_WIDTH};
pub use framebuffer::{FramebufferInfo, LinearFramebuffer};
pub use vrr_pacer::*;
pub use dock_manager::*;
pub mod panel_overdrive;
pub use panel_overdrive::*;

pub mod dsc_vrr;
pub use dsc_vrr::*;

pub mod dp_alt_mode;
pub use dp_alt_mode::*;

pub mod drrs_clock;
pub use drrs_clock::*;
