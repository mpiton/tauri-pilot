//! The webview windows the pilot server drives.
//!
//! Handlers reach webviews only through [`Webviews`]: [`TauriWebviews`] in the
//! app, and `fake::FakeWebviews` in handler tests.

use tauri::{Manager, Url};

/// Metadata of one window, as `windows.list` reports it.
///
/// Also the row shape of `error.data.available_windows`, which names the valid
/// labels when a request targets one that does not exist.
#[derive(Debug, serde::Serialize)]
pub(crate) struct WindowInfo {
    pub(crate) label: String,
    /// Empty when the runtime cannot report the URL.
    pub(crate) url: String,
    pub(crate) title: String,
}

/// The webview windows of the host app.
pub(crate) trait Webviews: Send + Sync {
    /// Resolve the target window: `label`, else `main`, else the first by label.
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

    /// Window label, as the host app knows it.
    #[cfg(any(test, feature = "press"))]
    fn label(&self) -> &str;

    /// Ask the OS to focus the window.
    ///
    /// # Errors
    ///
    /// Returns the runtime error when the focus request fails.
    #[cfg(feature = "press")]
    fn focus(&self) -> Result<(), String>;

    /// Whether the window currently has OS focus.
    ///
    /// [`Self::focus`] only reports that the activation request was dispatched.
    /// The window manager can ignore that request (X11 focus-stealing
    /// prevention) and still return success, so callers that inject OS-level
    /// input must poll this until it is true.
    ///
    /// On Windows this is the OS foreground window (or the root owner of
    /// `GetFocus`), not tao's parent-HWND `WM_SETFOCUS` flag. `WebView2` is a
    /// child HWND; when it holds keyboard focus the parent is blurred even
    /// though keys still land in the app.
    ///
    /// # Errors
    ///
    /// Returns the runtime error when the focus state cannot be queried.
    #[cfg(feature = "press")]
    fn is_focused(&self) -> Result<bool, String>;
}

/// [`Webviews`] backed by the Tauri app handle.
///
/// Windows are resolved on each call, so every request can target a
/// different window.
pub(crate) struct TauriWebviews<R: tauri::Runtime>(pub(crate) tauri::AppHandle<R>);

impl<R: tauri::Runtime> TauriWebviews<R> {
    /// Every webview window, in label order.
    ///
    /// Tauri returns a `HashMap`, whose order is not stable.
    fn windows(&self) -> std::collections::BTreeMap<String, tauri::WebviewWindow<R>> {
        self.0.webview_windows().into_iter().collect()
    }
}

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
impl<R: tauri::Runtime> TargetWindow for tauri::WebviewWindow<R> {
    fn url(&self) -> Option<Url> {
        current_url(self)
    }

    fn eval(&self, script: &str) -> Result<(), String> {
        Self::eval(self, script).map_err(|e| e.to_string())
    }

    #[cfg(any(test, feature = "press"))]
    fn label(&self) -> &str {
        Self::label(self)
    }

    #[cfg(feature = "press")]
    fn focus(&self) -> Result<(), String> {
        self.set_focus().map_err(|e| e.to_string())
    }

    #[cfg(all(feature = "press", not(windows)))]
    fn is_focused(&self) -> Result<bool, String> {
        Self::is_focused(self).map_err(|e| e.to_string())
    }

    #[cfg(all(feature = "press", windows))]
    fn is_focused(&self) -> Result<bool, String> {
        window_is_foreground_for_keys(self)
    }
}

/// Whether OS key events would land in `window`.
///
/// Tao's `is_focused` is parent-HWND `WM_SETFOCUS`. `WebView2` is a child, so
/// that flag is false while the app is still the foreground window and keys
/// still go to the webview. `set_focus` is also a no-op when already
/// foreground, so polling the tao flag cannot recover.
#[cfg(all(windows, feature = "press"))]
fn window_is_foreground_for_keys<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
) -> Result<bool, String> {
    let hwnd = window
        .hwnd()
        .map_err(|e| format!("cannot query window handle: {e}"))?;
    let window_bits = hwnd.0 as isize;
    // SAFETY: GetForegroundWindow / GetFocus / GetAncestor are user32
    // lookups. They do not dereference the HWND in user space.
    unsafe {
        let foreground = windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow();
        if foreground.0 as isize == window_bits {
            return Ok(true);
        }
        let focus = windows::Win32::UI::Input::KeyboardAndMouse::GetFocus();
        if focus.is_invalid() {
            return Ok(false);
        }
        let root = windows::Win32::UI::WindowsAndMessaging::GetAncestor(
            focus,
            windows::Win32::UI::WindowsAndMessaging::GA_ROOT,
        );
        Ok(root.0 as isize == window_bits)
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::{TargetWindow, Url, Webviews, WindowInfo};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    type FakeResponder = Arc<dyn Fn() + Send + Sync>;

    /// In-memory [`Webviews`] for handler tests.
    ///
    /// Without a label, `target` picks `main`, then the first window by label.
    /// Every evaluated script is recorded, then the responder runs: that is
    /// where a test plays the bridge and resolves the engine.
    /// `url()` is live: [`Self::set_url`] is visible to a `target` already
    /// held across an await, matching a real webview that navigated.
    ///
    /// [`Clone`] shares windows, scripts, and focus state, but starts with an
    /// empty responder. Sharing the responder would cycle when `on_eval`
    /// captures a clone of `self`.
    #[derive(Default)]
    pub(crate) struct FakeWebviews {
        windows: Arc<Mutex<BTreeMap<String, Option<Url>>>>,
        scripts: Arc<Mutex<Vec<String>>>,
        responder: Arc<Mutex<Option<FakeResponder>>>,
        /// Per-window OS focus. Missing labels report unfocused, so a `press`
        /// test cannot inject keys unless it opts in with [`Self::set_focused`].
        #[cfg(feature = "press")]
        focused: Arc<Mutex<BTreeMap<String, bool>>>,
        /// Per-window `is_focused` query failures. Takes precedence over
        /// [`Self::set_focused`].
        #[cfg(feature = "press")]
        focus_query_error: Arc<Mutex<BTreeMap<String, String>>>,
    }

    impl Clone for FakeWebviews {
        fn clone(&self) -> Self {
            Self {
                windows: Arc::clone(&self.windows),
                scripts: Arc::clone(&self.scripts),
                responder: Arc::new(Mutex::new(None)),
                #[cfg(feature = "press")]
                focused: Arc::clone(&self.focused),
                #[cfg(feature = "press")]
                focus_query_error: Arc::clone(&self.focus_query_error),
            }
        }
    }

    impl FakeWebviews {
        /// One window labeled `label`, showing `url`.
        pub(crate) fn window(label: &str, url: Option<&str>) -> Self {
            Self::windows(&[(label, url)])
        }

        /// Several windows labeled by name, each showing its optional URL.
        pub(crate) fn windows(windows: &[(&str, Option<&str>)]) -> Self {
            let windows = windows
                .iter()
                .map(|(label, url)| {
                    (
                        (*label).to_owned(),
                        url.map(|url| Url::parse(url).expect("valid test URL")),
                    )
                })
                .collect();
            Self {
                windows: Arc::new(Mutex::new(windows)),
                ..Self::default()
            }
        }

        /// Run `responder` after each eval.
        pub(crate) fn on_eval(self, responder: impl Fn() + Send + Sync + 'static) -> Self {
            *self.responder.lock().expect("responder mutex") = Some(Arc::new(responder));
            self
        }

        /// Point `label` at `url`. Visible to a live [`TargetWindow::url`].
        pub(crate) fn set_url(&self, label: &str, url: Option<&str>) {
            self.windows.lock().expect("windows mutex").insert(
                label.to_owned(),
                url.map(|url| Url::parse(url).expect("valid test URL")),
            );
        }

        /// Scripts evaluated so far, oldest first.
        pub(crate) fn scripts(&self) -> Vec<String> {
            self.scripts.lock().expect("scripts mutex").clone()
        }

        /// Report `focused` from [`TargetWindow::is_focused`] for `label`.
        /// Visible to a live [`TargetWindow`] already held across an await.
        /// Clears a previous [`Self::set_focus_query_error`] for `label`.
        #[cfg(feature = "press")]
        pub(crate) fn set_focused(&self, label: &str, focused: bool) {
            self.focused
                .lock()
                .expect("focused mutex")
                .insert(label.to_owned(), focused);
            self.focus_query_error
                .lock()
                .expect("focus query error mutex")
                .remove(label);
        }

        /// Report `Err` from [`TargetWindow::is_focused`] for `label`.
        /// Visible to a live [`TargetWindow`] already held across an await.
        #[cfg(feature = "press")]
        pub(crate) fn set_focus_query_error(&self, label: &str, error: impl Into<String>) {
            self.focus_query_error
                .lock()
                .expect("focus query error mutex")
                .insert(label.to_owned(), error.into());
        }
    }

    impl Webviews for FakeWebviews {
        fn target(&self, label: Option<&str>) -> Result<Box<dyn TargetWindow + '_>, String> {
            let windows = self.windows.lock().expect("windows mutex");
            let label = match label {
                Some(label) => windows
                    .contains_key(label)
                    .then(|| label.to_owned())
                    .ok_or_else(|| format!("Window '{label}' not found"))?,
                None => windows
                    .contains_key("main")
                    .then(|| "main".to_owned())
                    .or_else(|| windows.keys().next().cloned())
                    .ok_or_else(|| "No webview available".to_owned())?,
            };
            drop(windows);
            Ok(Box::new(FakeTarget {
                webviews: self,
                label,
            }))
        }

        fn list(&self) -> Vec<WindowInfo> {
            self.windows
                .lock()
                .expect("windows mutex")
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
        label: String,
    }

    impl TargetWindow for FakeTarget<'_> {
        fn url(&self) -> Option<Url> {
            self.webviews
                .windows
                .lock()
                .expect("windows mutex")
                .get(&self.label)
                .cloned()
                .flatten()
        }

        fn eval(&self, script: &str) -> Result<(), String> {
            self.webviews
                .scripts
                .lock()
                .expect("scripts mutex")
                .push(script.to_owned());
            let respond = self
                .webviews
                .responder
                .lock()
                .expect("responder mutex")
                .clone();
            if let Some(respond) = respond {
                respond();
            }
            Ok(())
        }

        #[cfg(any(test, feature = "press"))]
        fn label(&self) -> &str {
            &self.label
        }

        #[cfg(feature = "press")]
        fn focus(&self) -> Result<(), String> {
            Ok(())
        }

        #[cfg(feature = "press")]
        fn is_focused(&self) -> Result<bool, String> {
            if let Some(error) = self
                .webviews
                .focus_query_error
                .lock()
                .expect("focus query error mutex")
                .get(&self.label)
                .cloned()
            {
                return Err(error);
            }
            Ok(self
                .webviews
                .focused
                .lock()
                .expect("focused mutex")
                .get(&self.label)
                .copied()
                .unwrap_or(false))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{Arc, FakeWebviews, Mutex, Url, Webviews};

        fn with_windows(labels: &[&str]) -> FakeWebviews {
            let windows = labels
                .iter()
                .map(|label| {
                    let url =
                        Url::parse(&format!("https://{label}.test/")).expect("valid test URL");
                    ((*label).to_owned(), Some(url))
                })
                .collect();
            FakeWebviews {
                windows: Arc::new(Mutex::new(windows)),
                ..FakeWebviews::default()
            }
        }

        #[test]
        fn target_without_label_prefers_main() {
            let webviews = with_windows(&["settings", "main", "alpha"]);

            assert_eq!(
                webviews
                    .target(None)
                    .expect("main window")
                    .url()
                    .and_then(|url| url.host_str().map(str::to_owned)),
                Some("main.test".to_owned())
            );
        }

        #[test]
        fn target_without_main_uses_first_window_by_label() {
            let webviews = with_windows(&["zeta", "alpha"]);

            assert_eq!(
                webviews
                    .target(None)
                    .expect("first window")
                    .url()
                    .and_then(|url| url.host_str().map(str::to_owned)),
                Some("alpha.test".to_owned())
            );
        }

        #[test]
        fn target_uses_the_requested_label() {
            let webviews = with_windows(&["main", "settings"]);

            assert_eq!(
                webviews
                    .target(Some("settings"))
                    .expect("requested window")
                    .url()
                    .and_then(|url| url.host_str().map(str::to_owned)),
                Some("settings.test".to_owned())
            );
        }

        #[test]
        fn target_with_unknown_label_does_not_fall_back() {
            let webviews = with_windows(&["main"]);
            let result = webviews.target(Some("settings"));
            let Err(error) = result else {
                panic!("an unknown label must not choose another window")
            };

            assert_eq!(error, "Window 'settings' not found");
        }

        #[test]
        fn target_without_windows_errors() {
            let webviews = FakeWebviews::default();
            let result = webviews.target(None);
            let Err(error) = result else {
                panic!("a missing window should return an error")
            };

            assert_eq!(error, "No webview available");
        }

        #[test]
        fn url_is_live_after_set_url() {
            let webviews = FakeWebviews::window("main", Some("https://app.test/"));
            let target = webviews.target(None).expect("main window");
            webviews.set_url("main", Some("https://other.test/"));
            assert_eq!(
                target.url().map(|url| url.to_string()),
                Some("https://other.test/".to_owned())
            );
        }

        #[test]
        fn clone_shares_windows_not_responder() {
            use std::sync::atomic::{AtomicUsize, Ordering};

            let hits = Arc::new(AtomicUsize::new(0));
            let webviews = FakeWebviews::window("main", Some("https://app.test/"));
            let pages = webviews.clone();
            let hits_eval = Arc::clone(&hits);
            let webviews = webviews.on_eval(move || {
                hits_eval.fetch_add(1, Ordering::SeqCst);
                let _ = pages.target(None);
            });
            webviews
                .target(None)
                .expect("main window")
                .eval("1")
                .expect("eval");
            assert_eq!(hits.load(Ordering::SeqCst), 1);

            let later = webviews.clone();
            later
                .target(None)
                .expect("main window")
                .eval("2")
                .expect("eval");
            assert_eq!(
                hits.load(Ordering::SeqCst),
                1,
                "clone must start with an empty responder"
            );
        }

        #[test]
        fn list_sorts_windows_by_label() {
            let webviews = with_windows(&["settings", "main", "alpha"]);

            assert_eq!(
                webviews
                    .list()
                    .into_iter()
                    .map(|window| window.label)
                    .collect::<Vec<_>>(),
                ["alpha", "main", "settings"]
            );
        }

        #[test]
        fn target_label_matches_the_resolved_window() {
            let webviews = with_windows(&["settings", "main"]);
            assert_eq!(webviews.target(None).expect("main window").label(), "main");
            assert_eq!(
                webviews
                    .target(Some("settings"))
                    .expect("settings window")
                    .label(),
                "settings"
            );
        }

        #[cfg(feature = "press")]
        #[test]
        fn is_focused_is_live_after_set_focused() {
            let webviews = FakeWebviews::window("main", Some("https://app.test/"));
            let target = webviews.target(None).expect("main window");
            assert!(
                !target.is_focused().expect("focus query"),
                "a new fake window starts unfocused"
            );
            webviews.set_focused("main", true);
            assert!(
                target.is_focused().expect("focus query"),
                "set_focused must be visible on a held target"
            );
        }

        #[cfg(feature = "press")]
        #[test]
        fn clone_shares_focused_state() {
            let webviews = FakeWebviews::window("main", Some("https://app.test/"));
            let clone = webviews.clone();
            clone.set_focused("main", true);
            assert!(
                webviews
                    .target(None)
                    .expect("main window")
                    .is_focused()
                    .expect("focus query")
            );
        }

        #[cfg(feature = "press")]
        #[test]
        fn is_focused_returns_query_error() {
            let webviews = FakeWebviews::window("main", Some("https://app.test/"));
            webviews.set_focus_query_error("main", "FailedToSendMessage");
            let target = webviews.target(None).expect("main window");
            let err = target.is_focused().expect_err("query failed");
            assert_eq!(err, "FailedToSendMessage");
        }
    }
}

#[cfg(test)]
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
            // An existing data directory keeps tauri from creating the user's
            // real one during tests.
            WebviewWindowBuilder::new(&app, *name, WebviewUrl::External(url))
                .data_directory(std::env::temp_dir())
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
