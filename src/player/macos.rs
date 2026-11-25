use super::{ShieldCallback, shield};
use block2::RcBlock;
use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained};
use objc2_app_kit::NSView;
use objc2_foundation::{NSError, NSPoint, NSRect, NSSize, NSString};
use objc2_web_kit::{WKContentRuleList, WKContentRuleListStore};
use std::cell::RefCell;
use wry::{WebView, WebViewExtMacOS};

pub struct ClipContainer(Retained<NSView>);

impl ClipContainer {
    pub fn new(webview: &WebView) -> Result<Self, String> {
        let main_thread = MainThreadMarker::new().ok_or("AppKit requires the main thread.")?;
        let view = webview.webview();
        // Wry's attached view and its AppKit parent are alive on the main thread.
        let parent = unsafe { view.superview() }
            .ok_or("Could not find the embedded player’s parent view.")?;
        let container = NSView::initWithFrame(NSView::alloc(main_thread), view.frame());
        container.setWantsLayer(true);
        container
            .layer()
            .ok_or("Could not allocate the embedded player’s clipping layer.")?
            .setMasksToBounds(true);
        view.removeFromSuperview();
        parent.addSubview(&container);
        container.addSubview(&view);
        Ok(Self(container))
    }

    pub fn layout(&self, webview: &WebView, rect: eframe::egui::Rect, clip: eframe::egui::Rect) {
        // The container is retained by both this object and its parent during layout.
        let Some(parent) = (unsafe { self.0.superview() }) else {
            return;
        };
        let frame = |x, y, w, h| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
        let y = if parent.isFlipped() {
            f64::from(clip.top())
        } else {
            parent.bounds().size.height - f64::from(clip.bottom())
        };
        let clip_frame = frame(
            f64::from(clip.left()),
            y,
            f64::from(clip.width()),
            f64::from(clip.height()),
        );
        if self.0.frame() != clip_frame {
            self.0.setFrame(clip_frame);
        }
        let content = webview.webview();
        let content_frame = frame(
            f64::from(rect.left() - clip.left()),
            f64::from(clip.bottom() - rect.bottom()),
            f64::from(rect.width()),
            f64::from(rect.height()),
        );
        if content.frame() != content_frame {
            content.setFrame(content_frame);
        }
    }
}

impl Drop for ClipContainer {
    fn drop(&mut self) {
        self.0.removeFromSuperview();
    }
}

pub fn install_shield(webview: &WebView, complete: ShieldCallback) {
    let Some(main_thread) = MainThreadMarker::new() else {
        complete(Err("WebKit requires the macOS main thread.".to_owned()));
        return;
    };
    let Some(store) = (unsafe { WKContentRuleListStore::defaultStore(main_thread) }) else {
        complete(Err("WebKit could not open its rule store.".to_owned()));
        return;
    };
    let manager = webview.manager();
    let completion = RefCell::new(Some(complete));
    let block = RcBlock::new(move |rules: *mut WKContentRuleList, error: *mut NSError| {
        // WebKit owns the callback arguments for the duration of this call.
        let result = unsafe {
            if let Some(rules) = rules.as_ref() {
                manager.addContentRuleList(rules);
                Ok(())
            } else if let Some(error) = error.as_ref() {
                Err(error.localizedDescription().to_string())
            } else {
                Err("WebKit returned no content rules.".to_owned())
            }
        };
        if let Some(callback) = completion.borrow_mut().take() {
            callback(result);
        }
    });
    // WebKit copies the completion block while compiling the rules asynchronously.
    unsafe {
        store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
            Some(&NSString::from_str("me.watchvortex.desktop.shield")),
            Some(&NSString::from_str(&shield::content_rules())),
            Some(&block),
        );
    }
}
