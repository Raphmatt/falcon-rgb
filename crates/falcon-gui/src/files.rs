//! File dialogs that work on the desktop (`rfd`, blocking) and in the browser (download link,
//! async picker). Picked files arrive through an [`Inbox`] so both look the same to the UI.

use std::cell::RefCell;
use std::rc::Rc;

/// A picked file: (display name, contents) or an error.
pub type Picked = Result<(String, Vec<u8>), String>;

/// Receives the result of [`Inbox::open`], possibly a few frames later.
#[derive(Debug, Clone, Default)]
pub struct Inbox(Rc<RefCell<Option<Picked>>>);

impl Inbox {
    pub fn take(&self) -> Option<Picked> {
        self.0.borrow_mut().take()
    }

    /// Lets the user pick a file with one of `extensions`; `repaint` is called once it has
    /// been read.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(&self, title: &str, extensions: &[&str], repaint: impl FnOnce()) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(title)
            .add_filter("Falcon file", extensions)
            .pick_file()
        else {
            return;
        };
        let result = std::fs::read(&path)
            .map(|s| (path.display().to_string(), s))
            .map_err(|e| e.to_string());
        *self.0.borrow_mut() = Some(result);
        repaint();
    }

    #[cfg(target_arch = "wasm32")]
    pub fn open(&self, title: &str, extensions: &[&str], repaint: impl FnOnce() + 'static) {
        let slot = self.0.clone();
        let dialog = rfd::AsyncFileDialog::new()
            .set_title(title)
            .add_filter("Falcon file", extensions);
        wasm_bindgen_futures::spawn_local(async move {
            let Some(file) = dialog.pick_file().await else {
                return;
            };
            let result = Ok((file.file_name(), file.read().await));
            *slot.borrow_mut() = Some(result);
            repaint();
        });
    }
}

/// Saves `text` under a user-chosen path. Returns where it went, `None` if cancelled.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_text(title: &str, file_name: &str, text: &str) -> Result<Option<String>, String> {
    let Some(path) = rfd::FileDialog::new()
        .set_title(title)
        .set_file_name(file_name)
        .add_filter("JSON", &["json"])
        .save_file()
    else {
        return Ok(None);
    };
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
}

/// Offers `text` as a download named `file_name`.
#[cfg(target_arch = "wasm32")]
pub fn save_text(_title: &str, file_name: &str, text: &str) -> Result<Option<String>, String> {
    use wasm_bindgen::JsCast;
    let fail = |what: &str| format!("download failed ({what})");
    let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(text));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("application/json");
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts)
        .map_err(|_| fail("blob"))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|_| fail("url"))?;
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| fail("document"))?;
    let a: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .map_err(|_| fail("element"))?
        .dyn_into()
        .map_err(|_| fail("anchor"))?;
    a.set_href(&url);
    a.set_download(file_name);
    a.click();
    let _ = web_sys::Url::revoke_object_url(&url);
    Ok(Some(format!("your downloads ({file_name})")))
}

/// Path of an app picked by the user (for linked apps / launch keys). Desktop only.
#[cfg(not(target_arch = "wasm32"))]
pub fn pick_program(title: &str) -> Option<String> {
    let mut dlg = rfd::FileDialog::new().set_title(title);
    if cfg!(target_os = "macos") {
        dlg = dlg.set_directory("/Applications");
    }
    dlg.pick_file().map(|p| p.display().to_string())
}

/// Whether [`pick_program`] exists (not in the browser).
pub const CAN_PICK_PROGRAMS: bool = !cfg!(target_arch = "wasm32");

#[cfg(target_arch = "wasm32")]
pub fn pick_program(_title: &str) -> Option<String> {
    None
}
