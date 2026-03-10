use anyhow::Result;
use orinium_browser::browser::{BrowserApp, Tab};
use orinium_browser::platform::renderer::gpu::GpuRenderer;
use std::collections::HashMap;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

struct WindowSpec {
    title: &'static str,
    url: &'static str,
    size: (u32, u32),
}

const WINDOWS: &[WindowSpec] = &[
    WindowSpec {
        title: "Window 1 – Compatibility Test",
        url: "resource:///test/compatibility_test.html",
        size: (900, 640),
    },
    WindowSpec {
        title: "Window 2 – CSS Apply",
        url: "resource:///test/css_apply.html",
        size: (900, 640),
    },
    WindowSpec {
        title: "Window 3 – About",
        url: "resource:///about.html",
        size: (900, 640),
    },
];

struct WindowState {
    window: Arc<Window>,
    gpu_renderer: GpuRenderer,
}

struct MultiWindowApp {
    browser: BrowserApp,
    windows: HashMap<WindowId, WindowState>,
    /// Specs of windows not yet created (drained in `resumed`).
    pending_specs: Vec<(usize, WindowSpec)>,
}

impl MultiWindowApp {
    fn new() -> Result<Self> {
        let mut browser = BrowserApp::new((900, 640), "Orinium Browser".to_string());

        let mut pending_specs = Vec::new();
        for (i, spec) in WINDOWS.iter().enumerate() {
            let mut tab = Tab::new();
            tab.navigate(spec.url.parse()?);
            browser.add_tab(tab);
            // Store index alongside the spec data we need at window creation time.
            pending_specs.push((
                i,
                WindowSpec {
                    title: spec.title,
                    url: spec.url,
                    size: spec.size,
                },
            ));
        }

        Ok(Self {
            browser,
            windows: HashMap::new(),
            pending_specs,
        })
    }
}

impl ApplicationHandler for MultiWindowApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Create all pending windows. Each window is mapped to its own tab.
        for (tab_id, spec) in self.pending_specs.drain(..) {
            let window = Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_inner_size(winit::dpi::PhysicalSize::new(
                                spec.size.0,
                                spec.size.1,
                            ))
                            .with_title(spec.title),
                    )
                    .expect("failed to create window"),
            );
            let window_id = window.id();
            let scale_factor = window.scale_factor();

            let gpu_renderer = pollster::block_on(GpuRenderer::new(window.clone(), None))
                .expect("failed to create GPU renderer");

            self.browser.open_window(
                window_id,
                spec.size,
                spec.title.to_string(),
                scale_factor,
                tab_id,
            );

            let mut state = WindowState {
                window,
                gpu_renderer,
            };

            // Request initial draw.
            self.browser
                .apply_draw_commands(window_id, &mut state.gpu_renderer);
            state.window.request_redraw();

            self.windows.insert(window_id, state);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if !self.windows.contains_key(&window_id) {
            return;
        }

        let cmd = {
            let state = self.windows.get_mut(&window_id).unwrap();
            self.browser
                .handle_window_event(window_id, event, &mut state.gpu_renderer)
        };

        use orinium_browser::browser::BrowserCommand;
        match cmd {
            BrowserCommand::Exit => {
                self.windows.remove(&window_id);
                self.browser.close_window(window_id);
                if self.windows.is_empty() {
                    event_loop.exit();
                }
            }
            BrowserCommand::RequestRedraw => {
                if let Some(state) = self.windows.get(&window_id) {
                    state.window.request_redraw();
                    state
                        .window
                        .set_title(&self.browser.window_title(window_id));
                }
            }
            BrowserCommand::RenameWindowTitle => {
                if let Some(state) = self.windows.get(&window_id) {
                    state
                        .window
                        .set_title(&self.browser.window_title(window_id));
                }
            }
            BrowserCommand::None => {},
            BrowserCommand::OpenNewWindow { tab_id } => {
                let default_size = self.browser.default_window_size();
                let default_title = self.browser.default_window_title();
                let window = Arc::new(
                    event_loop
                        .create_window(
                            Window::default_attributes()
                                .with_inner_size(winit::dpi::PhysicalSize::new(
                                    default_size.0 as u32,
                                    default_size.1 as u32,
                                ))
                                .with_title(&default_title),
                        )
                        .expect("failed to create window"),
                );
                let new_id = window.id();
                let scale_factor = window.scale_factor();
                let gpu_renderer =
                    pollster::block_on(GpuRenderer::new(window.clone(), None))
                        .expect("failed to create GPU renderer");
                self.browser.open_window(
                    new_id,
                    (default_size.0 as u32, default_size.1 as u32),
                    default_title,
                    scale_factor,
                    tab_id,
                );
                let mut state = WindowState { window, gpu_renderer };
                self.browser.apply_draw_commands(new_id, &mut state.gpu_renderer);
                state.window.request_redraw();
                self.windows.insert(new_id, state);
            }
        }
    }
}

fn main() -> Result<()> {
    env_logger::init();

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

    let mut app = MultiWindowApp::new()?;
    event_loop.run_app(&mut app)?;

    Ok(())
}
