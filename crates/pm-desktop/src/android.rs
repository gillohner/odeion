//! Android-only setup.
//!
//! The pkarr relay client behind the Pubky Ring sign-in verifies TLS with
//! `rustls-platform-verifier`, which on Android needs the JVM and an app
//! context before the first handshake; without them the first handshake
//! panics ("Expect rustls-platform-verifier to be initialized"). Its Kotlin
//! classes come from the verifier's Maven archive, added to the generated
//! Gradle project by `tools/release/android-patch.py`.
//!
//! Tauri hands over `jni` 0.21 objects and the verifier uses `jni` 0.22, so
//! the two meet through raw pointers.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

static TLS_READY: AtomicBool = AtomicBool::new(false);
/// Whether [`init_tls`] succeeded: without the verifier, a TLS handshake panics.
static TLS_OK: AtomicBool = AtomicBool::new(false);

/// True once the TLS verifier is set up (after [`tls_ready`]); false if it failed or is late.
pub fn tls_ok() -> bool {
    TLS_OK.load(Ordering::Acquire)
}

#[cfg(feature = "ring-transport-probe")]
#[path = "ring_probe.rs"]
mod ring_probe;

/// Waits (at most 5 s) for [`init_tls`] to have run on the UI thread, so the first handshake
/// never finds the verifier missing (it panics then).
pub async fn tls_ready() {
    for _ in 0..100 {
        if TLS_READY.load(Ordering::Acquire) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    tracing::warn!("the TLS verifier is still not ready after 5 s");
}

/// Initialise the platform verifier from the webview's activity. Runs on
/// the activity's thread, long before anyone can press "Sign in".
pub fn init_tls(window: &tauri::WebviewWindow) {
    let scheduled = window.with_webview(|wv| {
        wv.jni_handle().exec(|env, activity, _webview| {
            let raw_env = env.get_raw() as *mut c_void;
            let raw_ctx = activity.as_raw() as *mut c_void;
            // SAFETY: both pointers belong to the live JNI frame of this
            // callback, on this thread.
            let ok = crate::crash::guarded("the TLS verifier", || unsafe { init_verifier(raw_env, raw_ctx) });
            if ok == Some(true) {
                TLS_OK.store(true, Ordering::Release);
                tracing::info!("TLS platform verifier ready");
            } else {
                tracing::error!("TLS platform verifier failed to initialise; Pubky Ring sign-in will fail");
            }
            TLS_READY.store(true, Ordering::Release);
            #[cfg(feature = "ring-transport-probe")]
            tauri::async_runtime::spawn(ring_probe::run());
        });
    });
    if let Err(e) = scheduled {
        tracing::error!("cannot reach the webview to set up TLS: {e}");
    }
}

/// # Safety
/// `raw_env` must be the current thread's `JNIEnv` and `raw_ctx` a live
/// local or global reference to an Android `Context`.
unsafe fn init_verifier(raw_env: *mut c_void, raw_ctx: *mut c_void) -> bool {
    // SAFETY: the caller guarantees a valid JNIEnv for this thread.
    let mut env = unsafe { jni::EnvUnowned::from_raw(raw_env as *mut jni::sys::JNIEnv) };
    let outcome = env.with_env(|env| -> Result<(), jni::errors::Error> {
        // SAFETY: the caller guarantees a live reference to a Context.
        let ctx = unsafe { jni::objects::JObject::from_raw(env, raw_ctx as jni::sys::jobject) };
        rustls_platform_verifier::android::init_with_env(env, ctx)
    });
    matches!(outcome.into_outcome(), jni::Outcome::Ok(()))
}

/// Android drops multicast packets unless an app holds a Wi-Fi multicast lock (it costs battery, so
/// only while looking for nodes). Needs `CHANGE_WIFI_MULTICAST_STATE` (tools/release/android-patch.py).
/// SSDP answers are unicast and arrive without it; mDNS answers need it.
pub struct MulticastLock {
    window: Option<tauri::WebviewWindow>,
}

/// The lock while held (one at a time).
static LOCK: std::sync::Mutex<Option<jni021::objects::GlobalRef>> = std::sync::Mutex::new(None);

impl MulticastLock {
    pub async fn acquire(app: &tauri::AppHandle) -> Self {
        use tauri::Manager;
        let Some(window) = app.get_webview_window(crate::MAIN) else {
            return Self { window: None };
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        let scheduled = window.with_webview(move |wv| {
            wv.jni_handle().exec(move |env, activity, _webview| {
                let taken = crate::crash::guarded("the Wi-Fi multicast lock", || take_lock(env, activity));
                let taken = taken.unwrap_or(Err(jni021::errors::Error::NullPtr("panicked")));
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                match taken {
                    Ok(lock) => {
                        if let Ok(mut held) = LOCK.lock() {
                            *held = Some(lock);
                        }
                    }
                    Err(e) => tracing::warn!("no Wi-Fi multicast lock ({e}); mDNS may find nothing"),
                }
                let _ = tx.send(());
            });
        });
        if scheduled.is_ok() {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), rx).await;
        }
        Self { window: Some(window) }
    }
}

impl Drop for MulticastLock {
    fn drop(&mut self) {
        let Some(window) = self.window.take() else { return };
        let _ = window.with_webview(|wv| {
            wv.jni_handle().exec(|env, _activity, _webview| {
                let held = LOCK.lock().ok().and_then(|mut l| l.take());
                if let Some(lock) = held {
                    let _ = env.call_method(lock.as_obj(), "release", "()V", &[]);
                }
            });
        });
    }
}

fn take_lock(
    env: &mut jni021::JNIEnv,
    activity: &jni021::objects::JObject,
) -> jni021::errors::Result<jni021::objects::GlobalRef> {
    use jni021::objects::JValue;
    let ctx = env
        .call_method(
            activity,
            "getApplicationContext",
            "()Landroid/content/Context;",
            &[],
        )?
        .l()?;
    let service = env.new_string("wifi")?;
    let wifi = env
        .call_method(
            &ctx,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&service)],
        )?
        .l()?;
    if wifi.is_null() {
        return Err(jni021::errors::Error::NullPtr("no Wi-Fi service"));
    }
    let tag = env.new_string("pubky-music-discovery")?;
    let lock = env
        .call_method(
            &wifi,
            "createMulticastLock",
            "(Ljava/lang/String;)Landroid/net/wifi/WifiManager$MulticastLock;",
            &[JValue::Object(&tag)],
        )?
        .l()?;
    env.call_method(&lock, "setReferenceCounted", "(Z)V", &[JValue::Bool(0)])?;
    env.call_method(&lock, "acquire", "()V", &[])?;
    env.new_global_ref(lock)
}

/// Start the player's foreground service (`PlayerService.kt`, added to the
/// generated project by tools/release/android-patch.py): it keeps the phone's
/// zone playing while the app is in the background. Must run while the app
/// is on screen; starting it again only updates its notification.
pub fn start_player_service(app: &tauri::AppHandle, text: String) {
    use tauri::Manager;
    let Some(window) = app.get_webview_window(crate::MAIN) else {
        return;
    };
    let scheduled = window.with_webview(move |wv| {
        wv.jni_handle().exec(move |env, activity, _webview| {
            let started = crate::crash::guarded("the player service", || start_service(env, activity, &text))
                .unwrap_or(Err(jni021::errors::Error::NullPtr("panicked")));
            match started {
                Ok(()) => tracing::info!("player service started"),
                Err(e) => {
                    // A Java exception stays pending until cleared.
                    if env.exception_check().unwrap_or(false) {
                        let _ = env.exception_describe();
                        let _ = env.exception_clear();
                    }
                    tracing::warn!(
                        "no player service ({e}); the phone's zone stops when the app is in the background"
                    )
                }
            }
        });
    });
    if let Err(e) = scheduled {
        tracing::warn!("cannot reach the activity to start the player service: {e}");
    }
}

fn start_service(
    env: &mut jni021::JNIEnv,
    activity: &jni021::objects::JObject,
    text: &str,
) -> jni021::errors::Result<()> {
    use jni021::objects::{JClass, JValue};
    // The service sits next to MainActivity, in its package. Loaded through
    // the app's class loader: JNI's FindClass may only see system classes.
    let class = env
        .call_method(activity, "getClass", "()Ljava/lang/Class;", &[])?
        .l()?;
    let name = env
        .call_method(&class, "getName", "()Ljava/lang/String;", &[])?
        .l()?;
    let name: String = env.get_string(&name.into())?.into();
    let service = match name.rsplit_once('.') {
        Some((pkg, _)) => format!("{pkg}.PlayerService"),
        None => "PlayerService".into(),
    };
    let loader = env
        .call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
        .l()?;
    let jname = env.new_string(&service)?;
    let cls = env
        .call_method(
            &loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&jname)],
        )?
        .l()?;
    let cls = JClass::from(cls);
    let jtext = env.new_string(text)?;
    env.call_static_method(
        &cls,
        "start",
        "(Landroid/app/Activity;Ljava/lang/String;)V",
        &[JValue::Object(activity), JValue::Object(&jtext)],
    )?;
    Ok(())
}

/// The share sheet for the crash report (`CrashReport.share`, tools/release/android/CrashReport.kt).
pub fn share_text(app: &tauri::AppHandle, text: String) -> Result<(), String> {
    use tauri::Manager;
    let window = app.get_webview_window(crate::MAIN).ok_or("no window")?;
    window
        .with_webview(move |wv| {
            wv.jni_handle().exec(move |env, activity, _webview| {
                let r = crate::crash::guarded("sharing the crash report", || {
                    call_static(env, activity, "CrashReport", "share", &text)
                });
                if !matches!(r, Some(Ok(()))) {
                    if env.exception_check().unwrap_or(false) {
                        let _ = env.exception_describe();
                        let _ = env.exception_clear();
                    }
                    tracing::warn!("could not open the share sheet");
                }
            });
        })
        .map_err(|e| e.to_string())
}

/// `<package>.<class>.<method>(Activity, String)`, loaded through the app's class loader.
fn call_static(
    env: &mut jni021::JNIEnv,
    activity: &jni021::objects::JObject,
    class: &str,
    method: &str,
    text: &str,
) -> jni021::errors::Result<()> {
    use jni021::objects::{JClass, JValue};
    let cls = env
        .call_method(activity, "getClass", "()Ljava/lang/Class;", &[])?
        .l()?;
    let name = env
        .call_method(&cls, "getName", "()Ljava/lang/String;", &[])?
        .l()?;
    let name: String = env.get_string(&name.into())?.into();
    let full = match name.rsplit_once('.') {
        Some((pkg, _)) => format!("{pkg}.{class}"),
        None => class.to_string(),
    };
    let loader = env
        .call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
        .l()?;
    let jname = env.new_string(&full)?;
    let target = env
        .call_method(
            &loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&jname)],
        )?
        .l()?;
    let target = JClass::from(target);
    let jtext = env.new_string(text)?;
    env.call_static_method(
        &target,
        method,
        "(Landroid/app/Activity;Ljava/lang/String;)V",
        &[JValue::Object(activity), JValue::Object(&jtext)],
    )?;
    Ok(())
}

/// The phone's part of app updates (`AppUpdate`, tools/release/android/AppUpdate.kt): its
/// methods block, so they are called from a blocking thread attached to the JVM, never from the
/// UI thread. Builds without that class (the .aab) have no updater.
pub struct Updater {
    vm: jni021::JavaVM,
    activity: jni021::objects::GlobalRef,
    class: jni021::objects::GlobalRef,
}

impl Updater {
    /// The updater, or `None` when this build has none.
    pub async fn get(app: &tauri::AppHandle) -> Option<std::sync::Arc<Self>> {
        use tauri::Manager;
        let window = app.get_webview_window(crate::MAIN)?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        window
            .with_webview(move |wv| {
                wv.jni_handle().exec(move |env, activity, _webview| {
                    let found = crate::crash::guarded("the updater", || updater(env, activity))
                        .unwrap_or(Err(jni021::errors::Error::NullPtr("panicked")));
                    if env.exception_check().unwrap_or(false) {
                        let _ = env.exception_clear();
                    }
                    let _ = tx.send(found.ok());
                });
            })
            .ok()?;
        rx.await.ok().flatten().map(std::sync::Arc::new)
    }

    fn call<T: Send + 'static>(
        self: &std::sync::Arc<Self>,
        f: impl FnOnce(
                &mut jni021::JNIEnv,
                &jni021::objects::JClass,
                &jni021::objects::JObject,
            ) -> jni021::errors::Result<T>
            + Send
            + 'static,
    ) -> impl std::future::Future<Output = anyhow::Result<T>> {
        let this = self.clone();
        async move {
            tauri::async_runtime::spawn_blocking(move || {
                let mut env = this.vm.attach_current_thread()?;
                let class: &jni021::objects::JClass = this.class.as_obj().into();
                let r = f(&mut *env, class, this.activity.as_obj());
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_describe();
                    let _ = env.exception_clear();
                }
                r
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .map_err(|e| anyhow::anyhow!("{e}"))
        }
    }

    /// The installed app's versionCode.
    pub async fn installed_version_code(self: &std::sync::Arc<Self>) -> anyhow::Result<u64> {
        use jni021::objects::JValue;
        let code = self
            .call(|env, class, activity| {
                env.call_static_method(
                    class,
                    "installedVersionCode",
                    "(Landroid/app/Activity;)J",
                    &[JValue::Object(activity)],
                )?
                .j()
            })
            .await?;
        u64::try_from(code).map_err(|_| anyhow::anyhow!("a negative versionCode"))
    }

    /// Asks; true for `yes`.
    pub async fn ask(self: &std::sync::Arc<Self>, title: &str, message: &str, yes: &str, no: &str) -> bool {
        use jni021::objects::JValue;
        let texts = [title, message, yes, no].map(str::to_owned);
        self.call(move |env, class, activity| {
            let [t, m, y, n] = texts;
            let (t, m, y, n) = (env.new_string(t)?, env.new_string(m)?, env.new_string(y)?, env.new_string(n)?);
            env.call_static_method(
                class,
                "ask",
                "(Landroid/app/Activity;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Z",
                &[
                    JValue::Object(activity),
                    JValue::Object(&t),
                    JValue::Object(&m),
                    JValue::Object(&y),
                    JValue::Object(&n),
                ],
            )?
            .z()
        })
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("updates: no question ({e})");
            false
        })
    }

    /// A short message at the bottom of the screen.
    pub async fn tell(self: &std::sync::Arc<Self>, message: &str) {
        use jni021::objects::JValue;
        let message = message.to_owned();
        let r = self
            .call(move |env, class, activity| {
                let m = env.new_string(message)?;
                env.call_static_method(
                    class,
                    "tell",
                    "(Landroid/app/Activity;Ljava/lang/String;)V",
                    &[JValue::Object(activity), JValue::Object(&m)],
                )?;
                Ok(())
            })
            .await;
        if let Err(e) = r {
            tracing::warn!("updates: no message ({e})");
        }
    }

    /// Hands the APK to Android's installer, which checks its SHA-256 once more. `Ok(None)` once
    /// Android has it (it asks the user to confirm), `Ok(Some(why))` when it was not installed.
    pub async fn install(
        self: &std::sync::Arc<Self>,
        path: &std::path::Path,
        sha256: &str,
    ) -> anyhow::Result<Option<String>> {
        use jni021::objects::JValue;
        let path = path.to_string_lossy().into_owned();
        let sha256 = sha256.to_owned();
        let why = self
            .call(move |env, class, activity| {
                let (p, h) = (env.new_string(path)?, env.new_string(sha256)?);
                let why = env
                    .call_static_method(
                        class,
                        "install",
                        "(Landroid/app/Activity;Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
                        &[JValue::Object(activity), JValue::Object(&p), JValue::Object(&h)],
                    )?
                    .l()?;
                let why: String = env.get_string(&why.into())?.into();
                Ok(why)
            })
            .await?;
        Ok((!why.is_empty()).then_some(why))
    }
}

/// The JVM, the activity and the `AppUpdate` class, loaded through the app's class loader.
fn updater(env: &mut jni021::JNIEnv, activity: &jni021::objects::JObject) -> jni021::errors::Result<Updater> {
    use jni021::objects::JValue;
    let cls = env
        .call_method(activity, "getClass", "()Ljava/lang/Class;", &[])?
        .l()?;
    let name = env
        .call_method(&cls, "getName", "()Ljava/lang/String;", &[])?
        .l()?;
    let name: String = env.get_string(&name.into())?.into();
    let full = match name.rsplit_once('.') {
        Some((pkg, _)) => format!("{pkg}.AppUpdate"),
        None => "AppUpdate".into(),
    };
    let loader = env
        .call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
        .l()?;
    let jname = env.new_string(&full)?;
    let class = env
        .call_method(
            &loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&jname)],
        )?
        .l()?;
    Ok(Updater {
        vm: env.get_java_vm()?,
        activity: env.new_global_ref(activity)?,
        class: env.new_global_ref(class)?,
    })
}

/// Tracing output for `adb logcat -s pubky-music` (Android drops stdout).
pub struct Logcat;

pub struct LogLine {
    prio: std::ffi::c_int,
    buf: Vec<u8>,
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logcat {
    type Writer = LogLine;

    fn make_writer(&'a self) -> LogLine {
        LogLine {
            prio: 4,
            buf: Vec::new(),
        }
    }

    fn make_writer_for(&'a self, meta: &tracing::Metadata<'_>) -> LogLine {
        // android/log.h: VERBOSE 2, DEBUG 3, INFO 4, WARN 5, ERROR 6.
        let prio = match *meta.level() {
            tracing::Level::TRACE => 2,
            tracing::Level::DEBUG => 3,
            tracing::Level::INFO => 4,
            tracing::Level::WARN => 5,
            tracing::Level::ERROR => 6,
        };
        LogLine {
            prio,
            buf: Vec::new(),
        }
    }
}

impl std::io::Write for LogLine {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(b);
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[link(name = "log")]
extern "C" {
    fn __android_log_write(
        prio: std::ffi::c_int,
        tag: *const std::ffi::c_char,
        text: *const std::ffi::c_char,
    ) -> std::ffi::c_int;
}

impl Drop for LogLine {
    fn drop(&mut self) {
        while self.buf.last() == Some(&b'\n') {
            self.buf.pop();
        }
        if self.buf.is_empty() {
            return;
        }
        self.buf.retain(|&c| c != 0);
        if let Ok(text) = std::ffi::CString::new(std::mem::take(&mut self.buf)) {
            // SAFETY: both are NUL-terminated strings that outlive the call.
            unsafe { __android_log_write(self.prio, c"pubky-music".as_ptr(), text.as_ptr()) };
        }
    }
}
