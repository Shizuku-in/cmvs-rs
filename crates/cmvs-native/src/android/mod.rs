//! Owned ANativeWindow handle for safe GPU surface construction.
use ndk::native_window::NativeWindow;
use raw_window_handle::{
    AndroidNdkWindowHandle, DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle,
    WindowHandle,
};

/// Retains the native window while a GPU surface borrows it. The host must drop
/// the surface before returning from Android's TerminateWindow callback.
pub struct AndroidWindow(NativeWindow);

impl AndroidWindow {
    pub fn new(window: NativeWindow) -> Self {
        Self(window)
    }

    pub fn dimensions(&self) -> [u32; 2] {
        [
            self.0.width().try_into().unwrap_or(0),
            self.0.height().try_into().unwrap_or(0),
        ]
    }
}

impl HasDisplayHandle for AndroidWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::android())
    }
}

impl HasWindowHandle for AndroidWindow {
    #[allow(unsafe_code)]
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let handle = AndroidNdkWindowHandle::new(self.0.ptr().cast());
        // SAFETY: NativeWindow owns an acquired ANativeWindow reference; the
        // returned handle cannot outlive the borrow of that owner.
        Ok(unsafe { WindowHandle::borrow_raw(handle.into()) })
    }
}

/// Schedule UI work without blocking Android's native lifecycle event loop.
#[allow(unsafe_code)]
pub fn show_font_picker(outer_app: &android_activity::AndroidApp) {
    let app = outer_app.clone();
    outer_app.run_on_java_main_thread(Box::new(move || {
        // SAFETY: AndroidApp owns the VM association and Activity global reference
        // throughout this closure; neither raw reference is deleted or retained.
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
        let result = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let raw: jni::sys::jobject = app.activity_as_ptr().cast();
            // SAFETY: borrow the Activity owned by app without taking ownership.
            let activity = unsafe { env.as_cast_raw::<jni::objects::JObject>(&raw)? };
            env.call_method(
                activity,
                jni::jni_str!("chooseFont"),
                jni::jni_sig!("()V"),
                &[],
            )?;
            Ok(())
        });
        if result.is_err()
            && let Some(directory) = app.internal_data_path()
        {
            let temporary = directory.join("font-result.tmp");
            if std::fs::write(&temporary, "cancel\n").is_ok() {
                let _ = std::fs::rename(temporary, directory.join("font-result.cfg"));
            }
        }
    }));
}
