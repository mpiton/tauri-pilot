//! The webview windows the pilot server drives.
//!
//! Handlers reach webviews only through [`Webviews`]: [`TauriWebviews`] in the
//! app, and `fake::FakeWebviews` in handler tests.

#[cfg(debug_assertions)]
use tauri::Manager;
use tauri::Url;

/// Metadata of one window, as `windows.list` reports it.
#[derive(Debug, serde::Serialize)]
pub(crate) struct WindowInfo {
    pub(crate) label: String,
    /// Empty when the runtime cannot report the URL.
    pub(crate) url: String,
    pub(crate) title: String,
}

/// The webview windows of the host app.
pub(crate) trait Webviews: Send + Sync {
    /// Resolve the target window: `label`, else `main`, else the first window.
    ///
    /// # Errors
    ///
    /// Returns an error when `label` names no window, since an explicit label
    /// never falls back to another window, or when the app has no window.
    fn target(&self, label: Option<&str>) -> Result<Box<dyn TargetWindow + '_>, String>;

    /// List every window, sorted by label.
    fn list(&self) -> Vec<WindowInfo>;
}

/// A window resolved by [`Webviews::target`].
///
/// `Send` so a handler can keep one across an `.await`.
pub(crate) trait TargetWindow: Send {
    /// URL of the current page, or `None` when the runtime cannot report it.
    fn url(&self) -> Option<Url>;

    /// Evaluate `script` in the current page without waiting for a result.
    ///
    /// Results come back through the `__callback` IPC command (ADR-001).
    ///
    /// # Errors
    ///
    /// Returns the runtime error when the script cannot be dispatched.
    fn eval(&self, script: &str) -> Result<(), String>;

    /// Ask the OS to focus the window.
    ///
    /// # Errors
    ///
    /// Returns the runtime error when the focus request fails.
    #[cfg(feature = "press")]
    fn focus(&self) -> Result<(), String>;
}

/// [`Webviews`] backed by the Tauri app handle.
///
/// Windows are resolved on each call, so every request can target a
/// different window.
#[cfg(debug_assertions)]
pub(crate) struct TauriWebviews<R: tauri::Runtime>(pub(crate) tauri::AppHandle<R>);

#[cfg(debug_assertions)]
impl<R: tauri::Runtime> TauriWebviews<R> {
    /// Every webview window, in label order.
    ///
    /// Tauri returns a `HashMap`, whose order is not stable.
    fn windows(&self) -> std::collections::BTreeMap<String, tauri::WebviewWindow<R>> {
        self.0.webview_windows().into_iter().collect()
    }
}

#[cfg(debug_assertions)]
impl<R: tauri::Runtime> Webviews for TauriWebviews<R> {
    fn target(&self, label: Option<&str>) -> Result<Box<dyn TargetWindow + '_>, String> {
        let window = match label {
            Some(label) => self
                .0
                .get_webview_window(label)
                .ok_or_else(|| format!("Window '{label}' not found"))?,
            None => self
                .0
                .get_webview_window("main")
                .or_else(|| self.windows().into_values().next())
                .ok_or_else(|| "No webview available".to_owned())?,
        };
        Ok(Box::new(window))
    }

    fn list(&self) -> Vec<WindowInfo> {
        self.windows()
            .into_iter()
            .map(|(label, window)| WindowInfo {
                url: TargetWindow::url(&window)
                    .map(|url| url.to_string())
                    .unwrap_or_default(),
                title: window.title().unwrap_or_default(),
                label,
            })
            .collect()
    }
}

/// Read the current URL of a webview, or `None` when the runtime cannot report it.
///
/// Also used by the `__callback` command to tag hellos with the page they
/// come from.
pub(crate) fn current_url<R: tauri::Runtime>(webview: &tauri::WebviewWindow<R>) -> Option<Url> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| webview.url()))
        .ok()
        .and_then(Result::ok)
}

// The bodies call the inherent `WebviewWindow` methods, which take precedence
// over these trait methods of the same name.
#[cfg(debug_assertions)]
impl<R: tauri::Runtime> TargetWindow for tauri::WebviewWindow<R> {
    fn url(&self) -> Option<Url> {
        current_url(self)
    }

    fn eval(&self, script: &str) -> Result<(), String> {
        Self::eval(self, script).map_err(|e| e.to_string())
    }

    #[cfg(feature = "press")]
    fn focus(&self) -> Result<(), String> {
        self.set_focus().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::{TargetWindow, Url, Webviews, WindowInfo};
    use std::sync::Mutex;

    /// In-memory [`Webviews`] for handler tests.
    ///
    /// Without a label, `target` picks the first window. Every evaluated
    /// script is recorded, then the responder runs: that is where a test
    /// plays the bridge and resolves the engine.
    #[derive(Default)]
    pub(crate) struct FakeWebviews {
        windows: Vec<(String, Option<Url>)>,
        scripts: Mutex<Vec<String>>,
        responder: Option<Box<dyn Fn() + Send + Sync>>,
    }

    impl FakeWebviews {
        /// One window labeled `label`, showing `url`.
        pub(crate) fn window(label: &str, url: Option<&str>) -> Self {
            let url = url.map(|url| Url::parse(url).expect("valid test URL"));
            Self {
                windows: vec![(label.to_owned(), url)],
                ..Self::default()
            }
        }

        /// Run `responder` after each eval.
        pub(crate) fn on_eval(mut self, responder: impl Fn() + Send + Sync + 'static) -> Self {
            self.responder = Some(Box::new(responder));
            self
        }

        /// Scripts evaluated so far, oldest first.
        pub(crate) fn scripts(&self) -> Vec<String> {
            self.scripts.lock().expect("scripts mutex").clone()
        }
    }

    impl Webviews for FakeWebviews {
        fn target(&self, label: Option<&str>) -> Result<Box<dyn TargetWindow + '_>, String> {
            let (_, url) = match label {
                Some(label) => self
                    .windows
                    .iter()
                    .find(|(name, _)| name == label)
                    .ok_or_else(|| format!("Window '{label}' not found"))?,
                None => self
                    .windows
                    .first()
                    .ok_or_else(|| "No webview available".to_owned())?,
            };
            Ok(Box::new(FakeTarget {
                webviews: self,
                url: url.clone(),
            }))
        }

        fn list(&self) -> Vec<WindowInfo> {
            self.windows
                .iter()
                .map(|(label, url)| WindowInfo {
                    label: label.clone(),
                    url: url.as_ref().map(Url::to_string).unwrap_or_default(),
                    title: String::new(),
                })
                .collect()
        }
    }

    struct FakeTarget<'a> {
        webviews: &'a FakeWebviews,
        url: Option<Url>,
    }

    impl TargetWindow for FakeTarget<'_> {
        fn url(&self) -> Option<Url> {
            self.url.clone()
        }

        fn eval(&self, script: &str) -> Result<(), String> {
            self.webviews
                .scripts
                .lock()
                .expect("scripts mutex")
                .push(script.to_owned());
            if let Some(respond) = &self.webviews.responder {
                respond();
            }
            Ok(())
        }

        #[cfg(feature = "press")]
        fn focus(&self) -> Result<(), String> {
            Ok(())
        }
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::{TauriWebviews, Webviews};
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    /// Resolve `label` in a mock app with one window per entry of `windows`,
    /// each showing `https://<label>.test/`, and return the host it shows.
    fn target_host(windows: &[&str], label: Option<&str>) -> Result<String, String> {
        let app = tauri::test::mock_app();
        for name in windows {
            let url = format!("https://{name}.test/")
                .parse()
                .expect("valid test URL");
            WebviewWindowBuilder::new(&app, *name, WebviewUrl::External(url))
                .build()
                .expect("build mock window");
        }
        let webviews = TauriWebviews(app.handle().clone());
        let url = webviews.target(label)?.url().expect("mock window URL");
        Ok(url.host_str().expect("test URL host").to_owned())
    }

    #[test]
    fn target_uses_the_requested_label() {
        let host = target_host(&["main", "settings"], Some("settings"));
        assert_eq!(host.as_deref(), Ok("settings.test"));
    }

    #[test]
    fn target_without_label_prefers_main() {
        // "alpha" sorts first, so only the `main` rule can pick main here.
        let host = target_host(&["alpha", "main"], None);
        assert_eq!(host.as_deref(), Ok("main.test"));
    }

    #[test]
    fn target_without_label_or_main_takes_the_first_by_label() {
        let host = target_host(&["beta", "alpha"], None);
        assert_eq!(host.as_deref(), Ok("alpha.test"));
    }

    #[test]
    fn target_with_unknown_label_does_not_fall_back() {
        let host = target_host(&["main"], Some("settings"));
        assert_eq!(host, Err("Window 'settings' not found".to_owned()));
    }

    #[test]
    fn target_without_windows_errors() {
        let host = target_host(&[], None);
        assert_eq!(host, Err("No webview available".to_owned()));
    }
}
