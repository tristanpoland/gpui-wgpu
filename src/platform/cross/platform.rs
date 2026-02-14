use crate::{
    BackgroundExecutor, DevicePixels, DummyKeyboardMapper, ForegroundExecutor, Platform,
    PlatformWindow, PriorityQueueReceiver, RunnableVariant, Size, WindowParams,
    platform::cross::{
        dispatcher::Dispatcher, keyboard::CrossKeyboardLayout, render_context::WgpuContext,
        text_system::CosmicTextSystem, window::CrossWindow,
    },
};
use anyhow::Result;
use collections::FxHashMap;
use std::{cell::Cell, rc::Rc, sync::Arc};
use winit::event_loop::ActiveEventLoop;

thread_local! {
    static ACTIVE_CONTEXT: Cell<Option<(*const ActiveEventLoop, *mut AppState)>> = Cell::new(None);
}

// Helper to access the context
fn with_active_context<R>(f: impl FnOnce(&ActiveEventLoop, &mut AppState) -> R) -> Option<R> {
    ACTIVE_CONTEXT.with(|storage| {
        let (loop_ptr, app_ptr) = storage.get()?;
        // SAFETY: We strictly manage these pointers during winit callbacks
        unsafe { Some(f(&*loop_ptr, &mut *app_ptr)) }
    })
}

enum CrossEvent {
    CreateWindow(CrossWindow, WindowParams),
}

pub(crate) struct CrossPlatform {
    background_executor: BackgroundExecutor,
    foreground_executor: ForegroundExecutor,
    text_system: Arc<CosmicTextSystem>,
    inner: CrossPlatformInner,
}

struct CrossPlatformInner {
    wgpu_context: Arc<WgpuContext>,
    state: CrossPlatformState,
    main_rx: PriorityQueueReceiver<RunnableVariant>,
    dispatcher: Arc<Dispatcher>,
    event_loop: Cell<Option<winit::event_loop::EventLoop<CrossEvent>>>,
    event_loop_proxy: winit::event_loop::EventLoopProxy<CrossEvent>,
}

struct CrossPlatformState {
    callbacks: Callbacks,
}

#[derive(Default)]
struct Callbacks {
    on_open_urls: Cell<Option<Box<dyn FnMut(Vec<String>)>>>,
    on_quit: Cell<Option<Box<dyn FnMut()>>>,
    on_reopen: Cell<Option<Box<dyn FnMut()>>>,
    on_app_menu_action: Cell<Option<Box<dyn FnMut(&dyn crate::Action)>>>,
    on_will_open_app_menu: Cell<Option<Box<dyn FnMut()>>>,
    on_validate_app_menu_command: Cell<Option<Box<dyn FnMut(&dyn crate::Action) -> bool>>>,
}

struct AppState {
    windows: FxHashMap<winit::window::WindowId, CrossWindow>,
    on_finish_launching: Cell<Option<Box<dyn 'static + FnOnce()>>>,
}

impl CrossPlatform {
    pub fn new() -> Result<Self> {
        let (main_tx, main_rx) = PriorityQueueReceiver::new();
        let dispatcher = Arc::new(Dispatcher::new(main_tx));
        let background_executor = BackgroundExecutor::new(dispatcher.clone());
        let foreground_executor = ForegroundExecutor::new(dispatcher.clone());

        let mut event_loop =
            winit::event_loop::EventLoop::<CrossEvent>::with_user_event().build()?;
        // TODO(mdeand): Can we use ControlFlow::Wait here?
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

        let event_loop_proxy = event_loop.create_proxy();

        Ok(Self {
            background_executor,
            foreground_executor,
            text_system: Arc::new(CosmicTextSystem::new()),
            inner: CrossPlatformInner {
                wgpu_context: Arc::new(WgpuContext::new()?),
                state: CrossPlatformState {
                    callbacks: Callbacks::default(),
                },
                main_rx,
                dispatcher,
                event_loop: Cell::new(Some(event_loop)),
                event_loop_proxy,
            },
        })
    }
}

impl Platform for CrossPlatform {
    fn background_executor(&self) -> BackgroundExecutor {
        self.background_executor.clone()
    }

    fn foreground_executor(&self) -> ForegroundExecutor {
        self.foreground_executor.clone()
    }

    fn text_system(&self) -> Arc<dyn crate::PlatformTextSystem> {
        self.text_system.clone()
    }

    fn run(&self, on_finish_launching: Box<dyn 'static + FnOnce()>) {
        let mut event_loop = self
            .inner
            .event_loop
            .take()
            .expect("App is already running");

        let mut app_state = AppState {
            windows: Default::default(),
            on_finish_launching: Cell::new(Some(on_finish_launching)),
        };

        event_loop
            .run_app(&mut app_state)
            .expect("Failed to run App");
    }

    fn quit(&self) {
        todo!()
    }

    fn restart(&self, binary_path: Option<std::path::PathBuf>) {
        todo!()
    }

    fn activate(&self, _ignoring_other_apps: bool) {}

    fn hide(&self) {
        todo!()
    }

    fn hide_other_apps(&self) {
        todo!()
    }

    fn unhide_other_apps(&self) {
        todo!()
    }

    fn displays(&self) -> Vec<Rc<dyn crate::PlatformDisplay>> {
        // TODO(mdeand): Add support for multiple displays.
        vec![]
    }

    fn primary_display(&self) -> Option<Rc<dyn crate::PlatformDisplay>> {
        // TODO(mdeand): Add support for multiple displays and primary display.
        None
    }

    fn active_window(&self) -> Option<crate::AnyWindowHandle> {
        // TODO(mdeand): Add support for tracking active window.
        None
    }

    fn open_window(
        &self,
        _handle: crate::AnyWindowHandle,
        options: crate::WindowParams,
    ) -> anyhow::Result<Box<dyn crate::PlatformWindow>> {
        let window = CrossWindow::new(self.inner.wgpu_context.clone());

        let success = with_active_context(|event_loop, app_state| {
            let attributes = winit::window::Window::default_attributes().with_title(
                options
                    .titlebar
                    .and_then(|t| t.title)
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "GPUI".into()),
            );

            let winit_window = event_loop
                .create_window(attributes)
                .expect("Failed to create window");
            let window_id = winit_window.id();

            window.initialize(winit_window);
            app_state.windows.insert(window_id, window.clone());
            window.window().request_redraw();
        })
        .is_some();

        if !success {
            anyhow::bail!("open_window called outside of main thread event loop");
        }

        Ok(Box::new(window))
    }

    fn window_appearance(&self) -> crate::WindowAppearance {
        todo!()
    }

    fn open_url(&self, url: &str) {
        todo!()
    }

    fn on_open_urls(&self, callback: Box<dyn FnMut(Vec<String>)>) {
        self.inner.state.callbacks.on_open_urls.set(Some(callback));
    }

    fn register_url_scheme(&self, url: &str) -> crate::Task<anyhow::Result<()>> {
        todo!()
    }

    fn prompt_for_paths(
        &self,
        _options: crate::PathPromptOptions,
    ) -> futures::channel::oneshot::Receiver<anyhow::Result<Option<Vec<std::path::PathBuf>>>> {
        todo!()
    }

    fn prompt_for_new_path(
        &self,
        _directory: &std::path::Path,
        _suggested_name: Option<&str>,
    ) -> futures::channel::oneshot::Receiver<anyhow::Result<Option<std::path::PathBuf>>> {
        todo!()
    }

    fn can_select_mixed_files_and_dirs(&self) -> bool {
        todo!()
    }

    fn reveal_path(&self, _path: &std::path::Path) {
        todo!()
    }

    fn open_with_system(&self, _path: &std::path::Path) {
        todo!()
    }

    fn on_quit(&self, callback: Box<dyn FnMut()>) {
        self.inner.state.callbacks.on_quit.set(Some(callback));
    }

    fn on_reopen(&self, callback: Box<dyn FnMut()>) {
        self.inner.state.callbacks.on_reopen.set(Some(callback));
    }

    fn set_menus(&self, menus: Vec<crate::Menu>, keymap: &crate::Keymap) {
        todo!()
    }

    fn set_dock_menu(&self, _menu: Vec<crate::MenuItem>, _keymap: &crate::Keymap) {
        todo!()
    }

    fn on_app_menu_action(&self, callback: Box<dyn FnMut(&dyn crate::Action)>) {
        self.inner
            .state
            .callbacks
            .on_app_menu_action
            .set(Some(callback));
    }

    fn on_will_open_app_menu(&self, callback: Box<dyn FnMut()>) {
        self.inner
            .state
            .callbacks
            .on_will_open_app_menu
            .set(Some(callback));
    }

    fn on_validate_app_menu_command(&self, callback: Box<dyn FnMut(&dyn crate::Action) -> bool>) {
        self.inner
            .state
            .callbacks
            .on_validate_app_menu_command
            .set(Some(callback));
    }

    fn app_path(&self) -> anyhow::Result<std::path::PathBuf> {
        todo!()
    }

    fn path_for_auxiliary_executable(&self, _name: &str) -> anyhow::Result<std::path::PathBuf> {
        todo!()
    }

    fn set_cursor_style(&self, _style: crate::CursorStyle) {
        todo!()
    }

    fn should_auto_hide_scrollbars(&self) -> bool {
        // TODO(mdeand): How do we want to implement this? For now, just return false.
        false
    }

    fn write_to_primary(&self, _item: crate::ClipboardItem) {
        // TODO(mdeand): clipboard-rs crate
        todo!()
    }

    fn write_to_clipboard(&self, _item: crate::ClipboardItem) {
        // TODO(mdeand): clipboard-rs crate
        todo!()
    }

    fn read_from_primary(&self) -> Option<crate::ClipboardItem> {
        // TODO(mdeand): clipboard-rs crate
        None
    }

    fn read_from_clipboard(&self) -> Option<crate::ClipboardItem> {
        // TODO(mdeand): clipboard-rs crate
        None
    }

    fn write_credentials(
        &self,
        url: &str,
        username: &str,
        password: &[u8],
    ) -> crate::Task<anyhow::Result<()>> {
        // TODO(mdeand): keyring crate
        todo!()
    }

    fn read_credentials(
        &self,
        url: &str,
    ) -> crate::Task<anyhow::Result<Option<(String, Vec<u8>)>>> {
        // TODO(mdeand): keyring crate
        todo!()
    }

    fn delete_credentials(&self, _url: &str) -> crate::Task<anyhow::Result<()>> {
        // TODO(mdeand): keyring crate
        todo!()
    }

    fn keyboard_layout(&self) -> Box<dyn crate::PlatformKeyboardLayout> {
        Box::new(CrossKeyboardLayout)
    }

    fn keyboard_mapper(&self) -> Rc<dyn crate::PlatformKeyboardMapper> {
        Rc::new(DummyKeyboardMapper)
    }

    fn on_keyboard_layout_change(&self, _callback: Box<dyn FnMut()>) {
        // TODO(mdeand): Is this possible to implement in a cross-platform way?
    }
}

impl winit::application::ApplicationHandler<CrossEvent> for AppState {
    fn new_events(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        cause: winit::event::StartCause,
    ) {
        let _ = (event_loop, cause);
    }

    fn user_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, event: CrossEvent) {
        match event {
            CrossEvent::CreateWindow(cross_window, params) => {
                let attributes = winit::window::Window::default_attributes().with_title(
                    params
                        .titlebar
                        .and_then(|x| x.title)
                        .as_ref()
                        .map(|x| x.as_ref())
                        .unwrap_or("gpui-wgpu"),
                );

                let winit_window = event_loop.create_window(attributes).unwrap();
                let window_id = winit_window.id();

                cross_window.initialize(winit_window);

                self.windows.insert(window_id, cross_window);
            }
        }
    }

    fn device_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        let _ = (event_loop, device_id, event);
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let _ = event_loop;
    }

    fn suspended(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let _ = event_loop;
    }

    fn exiting(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let _ = event_loop;
    }

    fn memory_warning(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let _ = event_loop;
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        ACTIVE_CONTEXT.with(|s| s.set(Some((event_loop as *const _, self as *mut _))));

        if let Some(on_finish_launching) = self.on_finish_launching.take() {
            on_finish_launching();
        }

        ACTIVE_CONTEXT.with(|s| s.set(None));
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        ACTIVE_CONTEXT.with(|s| s.set(Some((event_loop as *const _, self as *mut _))));

        let window = if let Some(w) = self.windows.get(&window_id) {
            w
        } else {
            return;
        };

        match event {
            winit::event::WindowEvent::Resized(physical_size) => {
                if physical_size.width == 0 || physical_size.height == 0 {
                    return;
                }

                if let Some(renderer) = window.0.renderer.get() {
                    renderer.borrow_mut().update_drawable_size(Size {
                        width: DevicePixels(physical_size.width as i32),
                        height: DevicePixels(physical_size.height as i32),
                    });
                }

                let scale_factor = window.scale_factor();
                let size = crate::Size {
                    width: crate::Pixels(physical_size.width as f32),
                    height: crate::Pixels(physical_size.height as f32),
                };

                if let Some(mut cb) = window.0.state.callbacks.on_resize.take() {
                    cb(size, scale_factor);
                    window.0.state.callbacks.on_resize.set(Some(cb));
                }
            }

            winit::event::WindowEvent::Moved(_) => {
                if let Some(mut cb) = window.0.state.callbacks.on_moved.take() {
                    cb();
                    window.0.state.callbacks.on_moved.set(Some(cb));
                }
            }

            winit::event::WindowEvent::Focused(active) => {
                if let Some(mut cb) = window.0.state.callbacks.on_active_status_change.take() {
                    cb(active);
                    window
                        .0
                        .state
                        .callbacks
                        .on_active_status_change
                        .set(Some(cb));
                }
            }

            winit::event::WindowEvent::ThemeChanged(_) => {
                if let Some(mut cb) = window.0.state.callbacks.on_appearance_changed.take() {
                    cb();
                    window.0.state.callbacks.on_appearance_changed.set(Some(cb));
                }
            }

            winit::event::WindowEvent::CloseRequested => {
                let should_close =
                    if let Some(mut cb) = window.0.state.callbacks.on_should_close.take() {
                        let result = cb();
                        window.0.state.callbacks.on_should_close.set(Some(cb));
                        result
                    } else {
                        true
                    };

                if should_close {
                    if let Some(cb) = window.0.state.callbacks.on_close.take() {
                        cb();
                    }
                    self.windows.remove(&window_id);
                }
            }

            winit::event::WindowEvent::RedrawRequested => {
                let physical_size = window.window().inner_size();
                if physical_size.width == 0 || physical_size.height == 0 {
                    return;
                }

                if let Some(mut cb) = window.0.state.callbacks.on_request_frame.take() {
                    cb(crate::RequestFrameOptions {
                        force_render: false,
                        require_presentation: true,
                    });
                    window.0.state.callbacks.on_request_frame.set(Some(cb));
                }
            }

            winit::event::WindowEvent::KeyboardInput { event, .. } => {
                // TODO(mdeand): Implement keyboard input handling.
                let _ = event;
            }

            _ => (),
        }

        ACTIVE_CONTEXT.with(|s| s.set(None));
    }
}
