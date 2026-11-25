use super::{ShieldCallback, shield};
use eframe::egui;
use std::{
    cell::RefCell,
    time::{Duration, Instant},
};
use webview2_com::{
    CoreWebView2EnvironmentOptions, CreateCoreWebView2EnvironmentCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL, COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
        CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2_22, ICoreWebView2Environment,
        ICoreWebView2EnvironmentOptions,
    },
    WebResourceRequestedEventHandler, take_pwstr,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows_core::{HSTRING, Interface, PCWSTR, PWSTR, w};
use wry::{WebView, WebViewExtWindows};

#[derive(Default)]
struct Runtime {
    started: Option<Instant>,
    result: Option<Result<ICoreWebView2Environment, String>>,
    com_initialized: bool,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        drop(self.result.take());
        if self.com_initialized {
            unsafe { CoUninitialize() };
        }
    }
}

thread_local! {
    static RUNTIME: RefCell<Runtime> = RefCell::default();
}

pub fn prepare(ctx: &egui::Context) -> Result<Option<ICoreWebView2Environment>, String> {
    let start = RUNTIME.with_borrow_mut(|runtime| {
        if runtime.started.is_some() {
            return false;
        }
        runtime.started = Some(Instant::now());
        true
    });
    if start && let Err(error) = start_runtime(ctx.clone()) {
        RUNTIME.with_borrow_mut(|runtime| {
            runtime.result = Some(Err(error.to_string()));
        });
    }
    RUNTIME.with_borrow(|runtime| match &runtime.result {
        Some(result) => result.clone().map(Some),
        None if runtime
            .started
            .is_some_and(|start| start.elapsed() >= Duration::from_secs(15)) =>
        {
            Err("The Windows player runtime took too long to start.".to_owned())
        }
        None => {
            ctx.request_repaint_after(Duration::from_millis(100));
            Ok(None)
        }
    })
}

fn start_runtime(ctx: egui::Context) -> windows_core::Result<()> {
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
    RUNTIME.with_borrow_mut(|runtime| runtime.com_initialized = true);
    let data = eframe::storage_dir(crate::APP_ID)
        .map(|path| HSTRING::from(path.join("player").as_path()))
        .unwrap_or_default();
    let options = CoreWebView2EnvironmentOptions::default();
    unsafe {
        options
            .set_additional_browser_arguments("--disable-features=msWebOOUI,msPdfOOUI".to_owned());
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            &data,
            &ICoreWebView2EnvironmentOptions::from(options),
            &CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
                move |status, environment| {
                    let result = status.map_err(|error| error.to_string()).and_then(|_| {
                        environment
                            .ok_or_else(|| "WebView2 did not return a player runtime.".to_owned())
                    });
                    RUNTIME.with_borrow_mut(|runtime| runtime.result = Some(result));
                    ctx.request_repaint();
                    Ok(())
                },
            )),
        )
    }
}

fn host_window(webview: &WebView) -> Result<windows::Win32::Foundation::HWND, String> {
    let mut window = windows::Win32::Foundation::HWND::default();
    // The controller returns the native child window Wry created for this player.
    unsafe { webview.controller().ParentWindow(&mut window) }.map_err(|error| error.to_string())?;
    Ok(window)
}

pub fn move_player(
    webview: &WebView,
    position: eframe::egui::Pos2,
    native_scale: f32,
) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
    };
    let window = host_window(webview)?;
    // A scroll only changes the position of the child. Leave WebView2's content size alone.
    unsafe {
        SetWindowPos(
            window,
            None,
            (position.x * native_scale).round() as i32,
            (position.y * native_scale).round() as i32,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOZORDER,
        )
    }
    .map_err(|error| error.to_string())
}

pub fn clip(
    webview: &WebView,
    bounds: eframe::egui::Rect,
    native_scale: f32,
) -> Result<(), String> {
    use windows::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject, SetWindowRgn};

    let pixel = |points: f32| (points * native_scale).round() as i32;
    let window = host_window(webview)?;
    unsafe {
        let region = CreateRectRgn(
            pixel(bounds.left()),
            pixel(bounds.top()),
            pixel(bounds.right()),
            pixel(bounds.bottom()),
        );
        if region.is_invalid() {
            return Err("Could not allocate a clipping region for the embedded player.".to_owned());
        }
        if SetWindowRgn(window, Some(region), true) == 0 {
            // SetWindowRgn owns the region after success.
            let _ = DeleteObject(region.into());
            return Err("Could not clip the embedded player to its page.".to_owned());
        }
    }
    Ok(())
}

pub fn install_shield(webview: &WebView, complete: ShieldCallback) {
    complete(install(webview).map_err(|error| error.to_string()));
}

fn install(webview: &WebView) -> windows_core::Result<()> {
    let core = webview.webview();
    let environment = webview.environment();
    // WebView2 owns the callback after registration. This view lives on the GUI thread.
    unsafe {
        if let Ok(core) = core.cast::<ICoreWebView2_22>() {
            core.AddWebResourceRequestedFilterWithRequestSourceKinds(
                w!("*"),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
            )?;
        } else {
            core.AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
        }
        core.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let mut uri = PWSTR::null();
                    args.Request()?.Uri(&mut uri)?;
                    if shield::blocks_request(&take_pwstr(uri)) {
                        let response = environment.CreateWebResourceResponse(
                            None,
                            403,
                            w!("Blocked by Vortex Shield"),
                            w!("Content-Length: 0\r\n"),
                        )?;
                        args.SetResponse(&response)?;
                    }
                }
                Ok(())
            })),
            &mut 0,
        )?;
    }
    Ok(())
}
