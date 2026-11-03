use copypasta::ClipboardProvider;
use copypasta::nop_clipboard::NopClipboardContext;
use winit::window::Window;

/// The system clipboard, plus the primary selection where the system has one.
pub struct Clipboard {
    main: Box<dyn ClipboardProvider>,
    primary: Option<Box<dyn ClipboardProvider>>,
}

impl Clipboard {
    #[cfg(target_os = "macos")]
    pub fn new(_window: &Window) -> Self {
        let main: Box<dyn ClipboardProvider> = match copypasta::ClipboardContext::new() {
            Ok(clipboard) => Box::new(clipboard),
            Err(_) => Box::new(NopClipboardContext),
        };
        Self { main, primary: None }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn new(window: &Window) -> Self {
        use copypasta::x11_clipboard::{Primary, X11ClipboardContext};
        use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

        if let Ok(RawDisplayHandle::Wayland(display)) = window.display_handle().map(|h| h.as_raw()) {
            // The Wayland clipboard rides on the window's own connection, so no XWayland is needed.
            // SAFETY: winit's own wl_display, alive as long as the event loop, which outlives every
            // window and its clipboard.
            let (primary, main) =
                unsafe { copypasta::wayland_clipboard::create_clipboards_from_external(display.display.as_ptr()) };
            return Self {
                main: Box::new(main),
                primary: Some(Box::new(primary)),
            };
        }
        let main: Box<dyn ClipboardProvider> = match copypasta::ClipboardContext::new() {
            Ok(clipboard) => Box::new(clipboard),
            Err(_) => Box::new(NopClipboardContext),
        };
        let primary = X11ClipboardContext::<Primary>::new()
            .ok()
            .map(|p| Box::new(p) as Box<dyn ClipboardProvider>);
        Self { main, primary }
    }

    pub fn get(&mut self) -> String {
        self.main.get_contents().unwrap_or_default()
    }

    pub fn set(&mut self, text: String) {
        let _ = self.main.set_contents(text);
    }

    pub fn get_primary(&mut self) -> Option<String> {
        self.primary.as_mut()?.get_contents().ok()
    }

    pub fn set_primary(&mut self, text: String) {
        if let Some(primary) = &mut self.primary {
            let _ = primary.set_contents(text);
        }
    }
}
