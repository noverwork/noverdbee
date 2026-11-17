//! Renders the real window offscreen and saves PNGs, so a change to the UI can
//! be looked at without a display. Needs a GPU: Metal on macOS, WGPU on Linux.
//!
//! `cargo test -p desktop --test screenshots` writes to `target/screenshots/`.

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("screenshots: skipped; no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shots {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use desktop_lib::ui::{self, store::Route};
    use gpui_kit::{
        px, size, AnyWindowHandle, Bounds, HeadlessAppContext, WindowBounds, WindowOptions,
    };

    struct Stage {
        cx: HeadlessAppContext,
        window: AnyWindowHandle,
        out: PathBuf,
    }

    impl Stage {
        /// Lets backend work finish and its results reach the views.
        fn settle(&mut self, duration: Duration) {
            let steps = (duration.as_millis() / 50).max(1);
            for _ in 0..steps {
                std::thread::sleep(Duration::from_millis(50));
                self.cx.run_until_parked();
            }
        }

        fn shoot(&mut self, name: &str) {
            self.cx
                .update_window(self.window, |_, window, cx| {
                    window.refresh();
                    let _ = window.draw(cx);
                })
                .ok();
            self.cx.run_until_parked();
            let image = self
                .cx
                .capture_screenshot(self.window)
                .expect("offscreen rendering must be available");
            let path = self.out.join(format!("{name}.png"));
            image.save(&path).expect("save screenshot");
            println!("screenshot: {}", path.display());
        }

        fn close_dialogs(&mut self) {
            use gpui_kit::component::WindowExt as _;
            self.cx
                .update_window(self.window, |_, window, cx| window.close_all_dialogs(cx))
                .ok();
            self.settle(Duration::from_millis(400));
        }

        fn store<R>(
            &mut self,
            update: impl FnOnce(&mut ui::store::Store, &mut gpui_kit::Context<ui::store::Store>) -> R,
        ) -> R {
            self.cx.update(|cx| ui::store::store(cx).update(cx, update))
        }
    }

    /// Opens the main window offscreen at `width` x `height` points.
    fn open_stage(
        app: &desktop_lib::app::AppHandle,
        out: PathBuf,
        width: f32,
        height: f32,
    ) -> Stage {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(ui::Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.allow_parking();
        let window = cx
            .update(|cx| {
                ui::init(app.clone(), cx);
                // Dialogs would otherwise be caught mid-entrance.
                cx.set_reduce_motion(true);
                ui::open_main_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: Default::default(),
                            size: size(px(width), px(height)),
                        })),
                        focus: false,
                        show: false,
                        ..Default::default()
                    },
                    cx,
                )
            })
            .expect("window opens");
        let mut stage = Stage { cx, window, out };
        stage.settle(Duration::from_millis(500));
        stage
    }

    pub fn run() {
        let data = tempfile::tempdir().expect("scratch data dir");
        std::env::set_var("NOVERDBEE_DATA_DIR", data.path());
        // Screenshots must not depend on what GitHub answers.
        std::env::set_var("NOVERDBEE_SKIP_UPDATE_CHECK", "1");
        let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
        std::fs::create_dir_all(&out).expect("screenshot dir");

        let app = desktop_lib::bootstrap::backend().expect("backend starts");
        let mut stage = open_stage(&app, out, 1200., 800.);

        stage.shoot("databases");

        stage.store(|store, cx| store.navigate(Route::Settings, cx));
        stage.settle(Duration::from_millis(500));
        stage.shoot("settings");

        let release = ui::views::updater::Release {
            version: "9.9.9".to_string(),
            notes: Some("- Faster table rendering\n- Fixes for wide results".to_string()),
            platforms: Default::default(),
        };
        stage
            .cx
            .update_window(stage.window, |_, window, cx| {
                ui::views::updater::show(release, window, cx)
            })
            .ok();
        stage.settle(Duration::from_millis(500));
        stage.shoot("update");
        stage.close_dialogs();
    }
}
