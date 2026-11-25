use super::{ShieldCallback, shield};
use gtk::{
    gio, glib,
    glib::translate::{ToGlibPtr, from_glib_full},
    prelude::*,
};
use std::{
    ffi::{CString, c_void},
    os::unix::ffi::OsStrExt,
    ptr,
};
use webkit2gtk::WebViewExt;
use webkit2gtk_sys as webkit;
use wry::{
    WebView, WebViewExtUnix,
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
};

pub fn initialize(frame: &eframe::Frame) -> Result<(), String> {
    let handle = frame.window_handle().map_err(|error| error.to_string())?;
    if !matches!(handle.as_raw(), RawWindowHandle::Xlib(_)) {
        return Err("Inline playback on Linux requires X11 or XWayland.".to_owned());
    }
    if !gtk::is_initialized() {
        gtk::gdk::set_allowed_backends("x11");
        gtk::init().map_err(|error| format!("Could not initialize the Linux player. {error}"))?;
    }
    Ok(())
}

fn host_widget(webview: &WebView) -> Result<gtk::Widget, String> {
    webview
        .webview()
        .toplevel()
        .ok_or_else(|| "Could not find the embedded GTK player window.".to_owned())
}

pub fn move_player(
    webview: &WebView,
    position: eframe::egui::Pos2,
    _native_scale: f32,
) -> Result<(), String> {
    let host = host_widget(webview)?;
    let window = host
        .downcast::<gtk::Window>()
        .map_err(|_| "Could not move the embedded GTK player window.")?;
    window.move_(position.x.round() as i32, position.y.round() as i32);
    Ok(())
}

pub fn clip(
    webview: &WebView,
    bounds: eframe::egui::Rect,
    _native_scale: f32,
) -> Result<(), String> {
    let widget = host_widget(webview)?;
    let region = gtk::cairo::Region::create_rectangle(&gtk::cairo::RectangleInt::new(
        bounds.left().round() as i32,
        bounds.top().round() as i32,
        bounds.width().round() as i32,
        bounds.height().round() as i32,
    ));
    widget.shape_combine_region(Some(&region));
    widget.input_shape_combine_region(Some(&region));
    Ok(())
}

pub fn pump_events() {
    for _ in 0..32 {
        if !gtk::events_pending() {
            break;
        }
        gtk::main_iteration_do(false);
    }
}

pub fn install_shield(webview: &WebView, complete: ShieldCallback) {
    let Some(manager) = webview.webview().user_content_manager() else {
        complete(Err("WebKitGTK has no content manager.".to_owned()));
        return;
    };
    let cache = glib::user_cache_dir().join("vortex-desktop/shield");
    if let Err(error) = std::fs::create_dir_all(&cache) {
        complete(Err(format!(
            "Could not create the Shield rule cache. {error}"
        )));
        return;
    }
    let Ok(path) = CString::new(cache.as_os_str().as_bytes()) else {
        complete(Err("The Shield cache directory is invalid.".to_owned()));
        return;
    };
    let rules = glib::Bytes::from_owned(shield::content_rules().into_bytes());

    // The async operation retains the store and rule bytes. Its callback owns Install.
    unsafe {
        let native_store = webkit::webkit_user_content_filter_store_new(path.as_ptr());
        if native_store.is_null() {
            complete(Err("WebKitGTK could not open its rule store.".to_owned()));
            return;
        }
        let _store: glib::Object =
            from_glib_full(native_store.cast::<glib::gobject_ffi::GObject>());
        let install = Box::new(Install { manager, complete });
        webkit::webkit_user_content_filter_store_save(
            native_store,
            c"vortex-shield".as_ptr(),
            rules.to_glib_none().0,
            ptr::null_mut(),
            Some(filter_ready),
            Box::into_raw(install).cast(),
        );
    }
}

struct Install {
    manager: webkit2gtk::UserContentManager,
    complete: ShieldCallback,
}

unsafe extern "C" fn filter_ready(
    store: *mut glib::gobject_ffi::GObject,
    result: *mut gio::ffi::GAsyncResult,
    data: *mut c_void,
) {
    // WebKit calls this once on the GTK thread with the original user data.
    unsafe {
        let Install { manager, complete } = *Box::from_raw(data.cast::<Install>());
        let mut error = ptr::null_mut();
        let filter =
            webkit::webkit_user_content_filter_store_save_finish(store.cast(), result, &mut error);
        if !filter.is_null() {
            webkit::webkit_user_content_manager_add_filter(manager.to_glib_none().0, filter);
            webkit::webkit_user_content_filter_unref(filter);
            complete(Ok(()));
        } else if !error.is_null() {
            let error: glib::Error = from_glib_full(error);
            complete(Err(error.to_string()));
        } else {
            complete(Err("WebKitGTK returned no content rules.".to_owned()));
        }
    }
}
