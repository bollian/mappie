use std::future::Future;
use std::sync::{Arc, Mutex};

use eyre::{Result, WrapErr, eyre};
use egui_winit::winit;
use egui_wgpu::wgpu;
use egui::Context;
use once_cell::sync::OnceCell;
use winit::event::{Event, WindowEvent};
use winit::event_loop::EventLoopProxy;

static ELOOP_PROXY: OnceCell<Mutex<EventLoopProxy<UserEvent>>> = OnceCell::new();
static ASYNC_RUNTIME: OnceCell<tokio::runtime::Runtime> = OnceCell::new();

fn proxy_send_best_effort(event: UserEvent) {
    let eloop = match ELOOP_PROXY.get() {
        Some(eloop) => eloop,
        None => {
            log::warn!("Early redraw request on nonexistant event loop");
            return
        },
    };

    let eloop = match eloop.lock() {
        Ok(eloop) => eloop,
        Err(e) => {
            log::warn!("Failed to acquire event loop proxy: {}", e);
            return
        }
    };

    match eloop.send_event(event) {
        Err(winit::event_loop::EventLoopClosed(_)) => {
            log::warn!("Event loop closed before task terminated. Assuming shutdown has commenced and ignoring.")
        }
        Ok(_) => {}
    }
}

pub fn request_redraw() {
    proxy_send_best_effort(UserEvent::RequestRedraw);
}

pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(async {
        let res = future.await;
        request_redraw();
        return res
    })
}

pub fn block_on<F>(future: F) -> F::Output
where
    F: Future + Send,
    F::Output: Send + 'static,
{
    let rt = ASYNC_RUNTIME.get()
        .expect("block_on called before async runtime initialization!");
    rt.block_on(future)
}

#[derive(Copy, Clone, Debug)]
enum UserEvent {
    RequestRedraw,
}

#[derive(Clone, Default)]
pub struct GamepadInput {
    pub dpad_down: bool,
    pub dpad_right: bool,
    pub dpad_up: bool,
    pub dpad_left: bool,

    pub left_stick: (f32, f32),
    pub right_stick: (f32, f32),
}

impl GamepadInput {
    fn button_state_mut(&mut self, button: gilrs::Button) -> Option<&mut bool> {
        Some(match button {
            gilrs::Button::DPadDown => &mut self.dpad_down,
            gilrs::Button::DPadRight => &mut self.dpad_right,
            gilrs::Button::DPadUp => &mut self.dpad_up,
            gilrs::Button::DPadLeft => &mut self.dpad_left,
            _ => return None,
        })
    }

    fn axis_state_mut(&mut self, axis: gilrs::Axis) -> Option<&mut f32> {
        Some(match axis {
            gilrs::Axis::LeftStickX => &mut self.left_stick.0,
            gilrs::Axis::LeftStickY => &mut self.left_stick.1,
            gilrs::Axis::RightStickX => &mut self.right_stick.0,
            gilrs::Axis::RightStickY => &mut self.right_stick.1,
            _ => return None,
        })
    }

    fn clear(&mut self) {
        self.dpad_down = false;
        self.dpad_right = false;
        self.dpad_up = false;
        self.dpad_left = false;
    }
}

pub trait App {
    type Err: std::error::Error + Send + Sync + 'static;
    fn update(&mut self, ctx: &Context, gamepad: GamepadInput);
}

struct GraphicsContext {
    window: Arc<winit::window::Window>,
    painter: egui_wgpu::winit::Painter,
    platform: egui_winit::State,
    pending_output: egui::FullOutput,
}

pub fn run<F, A>(main: F) -> Result<()>
where
    A: App + 'static,
    F: Future<Output = Result<A, A::Err>>,
{
    async fn graphics_constructor(event_loop: &winit::event_loop::EventLoopWindowTarget<UserEvent>)
        -> Result<GraphicsContext>
    {
        let window = Arc::new(winit::window::WindowBuilder::new()
            .with_decorations(true)
            .with_resizable(false)
            .with_transparent(false)
            .with_title("Mappie OI")
            .with_inner_size(winit::dpi::PhysicalSize {
                width: 1280,
                height: 800,
            })
            .build(&event_loop)
            .unwrap());
        let viewport_id = egui::ViewportId::from_hash_of(window.id());

        // We use the egui_wgpu_backend crate as the render backend.
        let mut painter = egui_wgpu::winit::Painter::new(egui_wgpu::WgpuConfiguration {
            on_surface_error: Arc::new(surface_error_callback),
            supported_backends: wgpu::Backends::default(),
            desired_maximum_frame_latency: None,
            power_preference: wgpu::PowerPreference::LowPower,
            present_mode: wgpu::PresentMode::Fifo,
            device_descriptor: Arc::new(|_adapter| {
                wgpu::DeviceDescriptor {
                    label: Some("device_descriptor"),
                    required_features: wgpu::Features::default(),
                    required_limits: wgpu::Limits::default(),
                }
            }),
        }, 1, None, false);
        painter.set_window(viewport_id, Some(Arc::clone(&window))).await
            .context("Failed to set WebGPU painter's window")?;

        let max_texture_side = painter.max_texture_side()
            .ok_or_else(|| eyre!("No maximum egui texture size provided"))?;

        let egui_ctx = egui::Context::default();
        let pending_output = egui::FullOutput::default();
        let mut platform = egui_winit::State::new(egui_ctx, viewport_id, &event_loop, None, None);
        platform.set_max_texture_side(max_texture_side);

        Ok(GraphicsContext {
            window,
            painter,
            platform,
            pending_output,
        })
    }

    let rt = ASYNC_RUNTIME.get_or_try_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
    }).wrap_err("Asynchronous runtime construction failed")?;
    let _enable_async_spawn = rt.enter();

    let mut app = rt.block_on(main)
        .map_err(|e| eyre!(e))
        .wrap_err("App construction failed")?;

    let mut gamepad_input_dirty = false;
    let mut gamepad = GamepadInput::default();
    let mut gilrs = gilrs::Gilrs::new()
        .map_err(|e| eyre!("Unable to acquire gamepad input context: {}", e))?;
    let mut graphics = once_cell::unsync::OnceCell::<GraphicsContext>::new();
    let event_loop = winit::event_loop::EventLoopBuilder::<UserEvent>::with_user_event().build()
        .context("Failed to acquire window event loop")?;
    ELOOP_PROXY.get_or_init(|| Mutex::new(event_loop.create_proxy()));

    event_loop.run(move |event, event_loop| {
        while let Some(gamepad_event) = gilrs.next_event() {
            match gamepad_event.event {
                gilrs::EventType::AxisChanged(axis, value, _) => {
                    if let Some(axis) = gamepad.axis_state_mut(axis) {
                        *axis = value;
                    }
                    gamepad_input_dirty = true;
                },
                gilrs::EventType::ButtonPressed(button, _) => {
                    if let Some(button) = gamepad.button_state_mut(button) {
                        *button = true;
                    }
                    gamepad_input_dirty = true;
                }
                gilrs::EventType::ButtonReleased(button, _) => {
                    if let Some(button) = gamepad.button_state_mut(button) {
                        *button = false;
                    }
                    gamepad_input_dirty = true;
                }
                _ => {}
            }
        }

        match event {
            Event::WindowEvent { event: window_event, .. } => {
                let graphics = match graphics.get_mut() {
                    Some(g) => g,
                    None => {
                        eprintln!("Graphics not initialized in time");
                        event_loop.exit();
                        return
                    }
                };

                let event_response = graphics.platform.on_window_event(&graphics.window, &window_event);
                if event_response.consumed {
                    // egui doesn't want us to respond to this event
                    return
                }
                if event_response.repaint {
                    graphics.window.request_redraw();
                }
                if !event_response.consumed {
                    match window_event {
                        WindowEvent::CloseRequested => {
                            event_loop.exit();
                        },
                        WindowEvent::RedrawRequested => {
                            let raw_input = graphics.platform.take_egui_input(&graphics.window);

                            // Call into user code and draw the window
                            let full_output = graphics.platform.egui_ctx().run(raw_input, |ctx| {
                                app.update(ctx, gamepad.clone())
                            });

                            gamepad.clear();
                            gamepad_input_dirty = false;
                            graphics.pending_output.append(full_output);
                            let egui::FullOutput {
                                platform_output,
                                textures_delta,
                                shapes,
                                pixels_per_point,
                                viewport_output
                            } = std::mem::take(&mut graphics.pending_output);

                            let clipped_primitives = graphics.platform.egui_ctx().tessellate(shapes, pixels_per_point);
                            let screenshot_requested = false;
                            for (&viewport, _output) in viewport_output.iter() {
                                let (_vsync_secs, _screenshot) = graphics.painter.paint_and_update_textures(
                                    viewport, pixels_per_point, [1.0, 0.0, 1.0, 0.0],
                                    &clipped_primitives,
                                    &textures_delta,
                                    screenshot_requested,
                                );
                            }

                            graphics.platform.handle_platform_output(
                                &graphics.window, platform_output);

                            // if repaint_after.is_zero() {
                            //     // requesting immediate repaint
                            //     *control_flow = ControlFlow::Poll;
                            // } else {
                                // let repaint_after = std::cmp::min(repaint_after, Duration::from_millis(33));
                                // println!("waiting {:?}", repaint_after);
                                // *control_flow = ControlFlow::WaitUntil(
                                //     std::time::Instant::now() + repaint_after
                                // );
                            // }
                        },
                        _ => {}
                    }
                }
            }
            Event::Resumed => {
                // lazy initialize the graphics context because it's required on Android
                match graphics.get_or_try_init(move || rt.block_on(graphics_constructor(event_loop))) {
                    Ok(_) => {},
                    Err(e) => {
                        eprintln!("Failed to initialize graphics: {}", e);
                        event_loop.exit();
                        return
                    }
                }
            }
            _ => {}
        }

        if let Some(graphics) = graphics.get() {
            if gamepad_input_dirty {
                graphics.window.request_redraw();
            }
        }
    }).context("Window event loop exited with error")
}

fn surface_error_callback(error: wgpu::SurfaceError) -> egui_wgpu::SurfaceErrorAction {
    use wgpu::SurfaceError::*;
    use egui_wgpu::SurfaceErrorAction;

    match error {
        Timeout => SurfaceErrorAction::SkipFrame,
        Outdated => SurfaceErrorAction::RecreateSurface,
        Lost => SurfaceErrorAction::RecreateSurface,
        OutOfMemory => {
            eprintln!("Out of memory for GUI frame");
            std::process::exit(255)
        }
    }
}
