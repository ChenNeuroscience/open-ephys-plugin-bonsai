//! Opening the live-view window from inside a host process (Open Ephys) that owns the main thread.
//!
//! winit allows one event loop per process, and eframe keeps it in a thread-local, so all windows
//! are run from a single long-lived GUI thread that waits for open requests.

use std::sync::Arc;

use crate::tracker::Shared;

#[cfg(target_os = "macos")]
pub fn open(_shared: Arc<Shared>) -> Result<(), String> {
    // AppKit only allows windows on the main thread, which JUCE owns. Use `tracker-dev` instead.
    Err("in-process window not supported on macOS".into())
}

/// Opens the window, or brings it to the front if it's already open (or about to be).
#[cfg(not(target_os = "macos"))]
pub fn open(shared: Arc<Shared>) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    use std::sync::OnceLock;

    use crossbeam_channel::Sender;

    static GUI_THREAD: OnceLock<Sender<Arc<Shared>>> = OnceLock::new();

    if shared.window_open.swap(true, Ordering::SeqCst) {
        if let Some(ctx) = shared.repaint.lock().unwrap().as_ref() {
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Focus);
        }
        return Ok(());
    }

    GUI_THREAD
        .get_or_init(|| {
            let (tx, rx) = crossbeam_channel::unbounded::<Arc<Shared>>();
            std::thread::Builder::new()
                .name("tracker-gui".into())
                .spawn(move || {
                    for shared in rx {
                        if shared.stop.load(std::sync::atomic::Ordering::Relaxed) {
                            continue; // acquisition already ended while we were busy
                        }
                        let mut options = crate::gui::default_options();
                        options.event_loop_builder = Some(Box::new(|builder| {
                            #[cfg(windows)]
                            winit::platform::windows::EventLoopBuilderExtWindows::with_any_thread(builder, true);
                            #[cfg(target_os = "linux")]
                            winit::platform::x11::EventLoopBuilderExtX11::with_any_thread(builder, true);
                        }));
                        if let Err(e) = crate::gui::run(shared.clone(), options) {
                            shared.set_problem(format!("window failed: {e}"));
                        }
                        shared.window_open.store(false, Ordering::SeqCst);
                    }
                })
                .expect("spawn gui thread");
            tx
        })
        .send(shared.clone())
        .map_err(|e| {
            shared.window_open.store(false, Ordering::SeqCst);
            e.to_string()
        })
}
