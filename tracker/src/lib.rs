//! Camera capture and LED tracking for the Open Ephys bonsai plugin, with a live-view window.
//! See PLAN.md at the repo root for the architecture.

pub mod camera;
pub mod config;
pub mod ffi;
pub mod gui;
pub mod pipeline;
pub mod tracker;
mod window;
