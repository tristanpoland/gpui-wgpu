use crate::{
    BackgroundExecutor, DevicePixels, DummyKeyboardMapper, ForegroundExecutor, Platform,
    PlatformWindow as _, PriorityQueueReceiver, RunnableVariant, Size,
    platform::cross::{
        dispatcher::{CrossEvent, Dispatcher},
        keyboard::CrossKeyboardLayout,
        render_context::WgpuContext,
        text_system::CosmicTextSystem,
        window::CrossWindow,
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

pub(crate) struct CrossPlatform {
    background_executor: BackgroundExecutor,
    foreground_executor: ForegroundExecutor,
    text_system: Arc<CosmicTextSystem>,
    wgpu_context: Arc<WgpuContext>,
    main_rx: PriorityQueueReceiver<RunnableVariant>,
    event_loop: Cell<Option<winit::event_loop::EventLoop<CrossEvent>>>,
    callbacks: PlatformCallbacks,
}

#[derive(Default)]
struct PlatformCallbacks {
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
    main_rx: PriorityQueueReceiver<RunnableVariant>,
}

impl CrossPlatform {
    pub fn new() -> Result<Self> {
        let (main_tx, main_rx) = PriorityQueueReceiver::new();
        let mut event_loop =
            winit::event_loop::EventLoop::<CrossEvent>::with_user_event().build()?;
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
        let event_loop_proxy = event_loop.create_proxy();

        let dispatcher = Arc::new(Dispatcher::new(main_tx, event_loop_proxy.clone()));
        let background_executor = BackgroundExecutor::new(dispatcher.clone());
        let foreground_executor = ForegroundExecutor::new(dispatcher.clone());

        Ok(Self {
            background_executor,
            foreground_executor,
            text_system: Arc::new(CosmicTextSystem::new()),
            wgpu_context: Arc::new(WgpuContext::new()?),
            main_rx,
            event_loop: Cell::new(Some(event_loop)),
            callbacks: PlatformCallbacks::default(),
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
        let mut event_loop = self.event_loop.take().expect("App is already running");

        let mut app_state = AppState {
            windows: Default::default(),
            on_finish_launching: Cell::new(Some(on_finish_launching)),
            main_rx: self.main_rx.clone(),
        };

        event_loop
            .run_app(&mut app_state)
            .expect("Failed to run App");
    }

    fn quit(&self) {
        // NOTE(mdeand): The event loop will exit when all windows are closed and there are no
        // NOTE(mdeand): more events to process. For an explicit quit, we rely on winit's exit
        // NOTE(mdeand): mechanism via the ActiveEventLoop.
        with_active_context(|event_loop, _| {
            event_loop.exit();
        });
    }

    fn restart(&self, _binary_path: Option<std::path::PathBuf>) {
        log::warn!("restart is not yet implemented on this platform");
    }

    fn activate(&self, _ignoring_other_apps: bool) {}

    fn hide(&self) {
        log::warn!("hide is not yet implemented on this platform");
    }

    fn hide_other_apps(&self) {
        log::warn!("hide_other_apps is not yet implemented on this platform");
    }

    fn unhide_other_apps(&self) {
        log::warn!("unhide_other_apps is not yet implemented on this platform");
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
        let window = CrossWindow::new(self.wgpu_context.clone());

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
        crate::WindowAppearance::default()
    }

    fn open_url(&self, _url: &str) {
        log::warn!("open_url is not yet implemented on this platform");
    }

    fn on_open_urls(&self, callback: Box<dyn FnMut(Vec<String>)>) {
        self.callbacks.on_open_urls.set(Some(callback));
    }

    fn register_url_scheme(&self, _url: &str) -> crate::Task<anyhow::Result<()>> {
        crate::Task::ready(Err(anyhow::anyhow!(
            "register_url_scheme is not yet implemented on this platform"
        )))
    }

    fn prompt_for_paths(
        &self,
        _options: crate::PathPromptOptions,
    ) -> futures::channel::oneshot::Receiver<anyhow::Result<Option<Vec<std::path::PathBuf>>>> {
        let (sender, receiver) = futures::channel::oneshot::channel();
        let _ = sender.send(Ok(None));
        receiver
    }

    fn prompt_for_new_path(
        &self,
        _directory: &std::path::Path,
        _suggested_name: Option<&str>,
    ) -> futures::channel::oneshot::Receiver<anyhow::Result<Option<std::path::PathBuf>>> {
        let (sender, receiver) = futures::channel::oneshot::channel();
        let _ = sender.send(Ok(None));
        receiver
    }

    fn can_select_mixed_files_and_dirs(&self) -> bool {
        false
    }

    fn reveal_path(&self, _path: &std::path::Path) {
        log::warn!("reveal_path is not yet implemented on this platform");
    }

    fn open_with_system(&self, _path: &std::path::Path) {
        log::warn!("open_with_system is not yet implemented on this platform");
    }

    fn on_quit(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.on_quit.set(Some(callback));
    }

    fn on_reopen(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.on_reopen.set(Some(callback));
    }

    fn set_menus(&self, _menus: Vec<crate::Menu>, _keymap: &crate::Keymap) {}

    fn set_dock_menu(&self, _menu: Vec<crate::MenuItem>, _keymap: &crate::Keymap) {}

    fn on_app_menu_action(&self, callback: Box<dyn FnMut(&dyn crate::Action)>) {
        self.callbacks.on_app_menu_action.set(Some(callback));
    }

    fn on_will_open_app_menu(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.on_will_open_app_menu.set(Some(callback));
    }

    fn on_validate_app_menu_command(&self, callback: Box<dyn FnMut(&dyn crate::Action) -> bool>) {
        self.callbacks
            .on_validate_app_menu_command
            .set(Some(callback));
    }

    fn app_path(&self) -> anyhow::Result<std::path::PathBuf> {
        Ok(std::env::current_exe()?)
    }

    fn path_for_auxiliary_executable(&self, _name: &str) -> anyhow::Result<std::path::PathBuf> {
        Err(anyhow::anyhow!(
            "path_for_auxiliary_executable is not yet implemented on this platform"
        ))
    }

    fn set_cursor_style(&self, _style: crate::CursorStyle) {}

    fn should_auto_hide_scrollbars(&self) -> bool {
        // TODO(mdeand): How do we want to implement this? For now, just return false.
        false
    }

    fn write_to_primary(&self, _item: crate::ClipboardItem) {
        log::warn!("write_to_primary is not yet implemented on this platform");
    }

    fn write_to_clipboard(&self, _item: crate::ClipboardItem) {
        log::warn!("write_to_clipboard is not yet implemented on this platform");
    }

    fn read_from_primary(&self) -> Option<crate::ClipboardItem> {
        None
    }

    fn read_from_clipboard(&self) -> Option<crate::ClipboardItem> {
        None
    }

    fn write_credentials(
        &self,
        _url: &str,
        _username: &str,
        _password: &[u8],
    ) -> crate::Task<anyhow::Result<()>> {
        crate::Task::ready(Err(anyhow::anyhow!(
            "write_credentials is not yet implemented on this platform"
        )))
    }

    fn read_credentials(
        &self,
        _url: &str,
    ) -> crate::Task<anyhow::Result<Option<(String, Vec<u8>)>>> {
        crate::Task::ready(Err(anyhow::anyhow!(
            "read_credentials is not yet implemented on this platform"
        )))
    }

    fn delete_credentials(&self, _url: &str) -> crate::Task<anyhow::Result<()>> {
        crate::Task::ready(Err(anyhow::anyhow!(
            "delete_credentials is not yet implemented on this platform"
        )))
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

impl AppState {
    fn set_active_context(&mut self, event_loop: &ActiveEventLoop) {
        ACTIVE_CONTEXT.with(|s| s.set(Some((event_loop as *const _, self as *mut _))));
    }

    fn clear_active_context(&self) {
        ACTIVE_CONTEXT.with(|s| s.set(None));
    }

    fn drain_main_queue(&mut self) {
        while let Ok(Some(runnable)) = self.main_rx.try_pop() {
            match runnable {
                RunnableVariant::Compat(runnable) => {
                    runnable.run();
                }
                RunnableVariant::Meta(_) => unimplemented!(),
            }
        }
    }
}

impl winit::application::ApplicationHandler<CrossEvent> for AppState {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _cause: winit::event::StartCause) {}

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: CrossEvent) {
        self.set_active_context(event_loop);

        match event {
            CrossEvent::WakeUp => {
                self.drain_main_queue();
            }
        }

        self.clear_active_context();
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        _event: winit::event::DeviceEvent,
    ) {
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.set_active_context(event_loop);

        self.drain_main_queue();

        for window in self.windows.values() {
            window.window().request_redraw();
        }

        self.clear_active_context();
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {}

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {}

    fn memory_warning(&mut self, _event_loop: &ActiveEventLoop) {}

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.set_active_context(event_loop);

        if let Some(on_finish_launching) = self.on_finish_launching.take() {
            on_finish_launching();
        }

        self.clear_active_context();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        self.set_active_context(event_loop);

        let Some(window) = self.windows.get(&window_id) else {
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

                window
                    .0
                    .state
                    .callbacks
                    .invoke_mut(&window.0.state.callbacks.on_resize, |cb| {
                        cb(size, scale_factor);
                    });
            }

            winit::event::WindowEvent::Moved(_) => {
                window
                    .0
                    .state
                    .callbacks
                    .invoke_mut(&window.0.state.callbacks.on_moved, |cb| {
                        cb();
                    });
            }

            winit::event::WindowEvent::Focused(active) => {
                window
                    .0
                    .state
                    .callbacks
                    .invoke_mut(&window.0.state.callbacks.on_active_status_change, |cb| {
                        cb(active)
                    });
            }

            winit::event::WindowEvent::ThemeChanged(_) => {
                window
                    .0
                    .state
                    .callbacks
                    .invoke_mut(&window.0.state.callbacks.on_appearance_changed, |cb| cb());
            }

            winit::event::WindowEvent::CloseRequested => {
                let should_close = window
                    .0
                    .state
                    .callbacks
                    .on_should_close
                    .take()
                    .map(|mut cb| {
                        let result = cb();
                        window.0.state.callbacks.on_should_close.set(Some(cb));
                        result
                    })
                    .unwrap_or(true);

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

                window.0.state.callbacks.invoke_mut(
                    &window.0.state.callbacks.on_request_frame,
                    |cb| {
                        cb(crate::RequestFrameOptions {
                            force_render: false,
                            require_presentation: true,
                        });
                    },
                );
            }

            winit::event::WindowEvent::KeyboardInput { .. } => {}

            _ => (),
        }

        self.clear_active_context();
    }
}
