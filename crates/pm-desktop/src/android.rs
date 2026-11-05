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

/// Watch the USB DACs attached to the phone (`UsbAudio`, tools/release/android/UsbAudio.kt): its
/// callback starts on the UI thread, then a thread of ours, attached to the JVM, waits for each new
/// report, describes the DAC (`pm_out_local::usb_dac`) for the phone's output and calls `changed`
/// when that is different. Builds without the class watch nothing.
pub fn watch_usb_audio(app: &tauri::AppHandle, changed: impl Fn() + Send + 'static) {
    use tauri::Manager;
    let Some(window) = app.get_webview_window(crate::MAIN) else {
        return;
    };
    let scheduled = window.with_webview(move |wv| {
        wv.jni_handle().exec(move |env, activity, _webview| {
            let started = crate::crash::guarded("the USB audio watcher", || start_usb_audio(env, activity))
                .unwrap_or(Err(jni021::errors::Error::NullPtr("panicked")));
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_describe();
                let _ = env.exception_clear();
            }
            let (vm, class) = match started {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("no USB DAC detection ({e})");
                    return;
                }
            };
            // The bit-perfect player (BitPerfect.kt), for the phone's output
            match crate::crash::guarded("the bit-perfect player", || {
                app_class(env, activity, "BitPerfect")
            })
            .unwrap_or(Err(jni021::errors::Error::NullPtr("panicked")))
            {
                Ok(bp) => match env.get_java_vm() {
                    Ok(vm2) => pm_out_local::bit_perfect::set_backend(std::sync::Arc::new(JniBitPerfect {
                        vm: std::sync::Arc::new(vm2),
                        class: bp,
                    })),
                    Err(e) => tracing::warn!("no bit-perfect player: {e}"),
                },
                Err(e) => {
                    if env.exception_check().unwrap_or(false) {
                        let _ = env.exception_clear();
                    }
                    tracing::warn!("no bit-perfect player ({e})");
                }
            }
            // Exclusive USB (UsbExclusive.kt): the app's own driver for the phone's DAC
            match crate::crash::guarded("the exclusive USB driver", || {
                app_class(env, activity, "UsbExclusive")
            })
            .unwrap_or(Err(jni021::errors::Error::NullPtr("panicked")))
            {
                Ok(c) => match env.get_java_vm() {
                    Ok(vm3) => {
                        pm_out_local::exclusive::set_backend(std::sync::Arc::new(JniUsbExclusive {
                            vm: std::sync::Arc::new(vm3),
                            class: c,
                        }));
                        // A DAC a killed app left without Android's driver goes back to it
                        let _ = std::thread::Builder::new()
                            .name("usb-recover".into())
                            .spawn(pm_out_local::exclusive::recover);
                    }
                    Err(e) => tracing::warn!("no exclusive USB driver: {e}"),
                },
                Err(e) => {
                    if env.exception_check().unwrap_or(false) {
                        let _ = env.exception_clear();
                    }
                    tracing::warn!("no exclusive USB driver ({e})");
                }
            }
            let spawned = std::thread::Builder::new()
                .name("usb-audio".into())
                .spawn(move || usb_audio_loop(vm, class, changed));
            if let Err(e) = spawned {
                tracing::warn!("no USB DAC detection: {e}");
            }
        });
    });
    if let Err(e) = scheduled {
        tracing::warn!("cannot reach the activity to watch USB DACs: {e}");
    }
}

/// `UsbAudio.start(activity)`, and what the waiting thread needs: the JVM and the class.
fn start_usb_audio(
    env: &mut jni021::JNIEnv,
    activity: &jni021::objects::JObject,
) -> jni021::errors::Result<(jni021::JavaVM, jni021::objects::GlobalRef)> {
    use jni021::objects::{JClass, JValue};
    let cls = env
        .call_method(activity, "getClass", "()Ljava/lang/Class;", &[])?
        .l()?;
    let name = env
        .call_method(&cls, "getName", "()Ljava/lang/String;", &[])?
        .l()?;
    let name: String = env.get_string(&name.into())?.into();
    let full = match name.rsplit_once('.') {
        Some((pkg, _)) => format!("{pkg}.UsbAudio"),
        None => "UsbAudio".into(),
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
    let class = JClass::from(class);
    env.call_static_method(
        &class,
        "start",
        "(Landroid/app/Activity;)V",
        &[JValue::Object(activity)],
    )?;
    Ok((env.get_java_vm()?, env.new_global_ref(&class)?))
}

/// `<package>.<name>` through the activity's class loader, as a global reference.
fn app_class(
    env: &mut jni021::JNIEnv,
    activity: &jni021::objects::JObject,
    name: &str,
) -> jni021::errors::Result<jni021::objects::GlobalRef> {
    use jni021::objects::JValue;
    let cls = env
        .call_method(activity, "getClass", "()Ljava/lang/Class;", &[])?
        .l()?;
    let full = env
        .call_method(&cls, "getName", "()Ljava/lang/String;", &[])?
        .l()?;
    let full: String = env.get_string(&full.into())?.into();
    let full = match full.rsplit_once('.') {
        Some((pkg, _)) => format!("{pkg}.{name}"),
        None => name.to_string(),
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
    env.new_global_ref(class)
}

/// The bit-perfect player (`BitPerfect.kt`) for pm-out-local's `bit_perfect`: its calls run on the
/// zone's audio thread, attached to the JVM for good (it plays for as long as the app runs).
struct JniBitPerfect {
    vm: std::sync::Arc<jni021::JavaVM>,
    class: jni021::objects::GlobalRef,
}

/// Run `f` with the JVM on this thread and a local frame, a pending Java exception cleared.
fn with_jvm<T>(
    vm: &jni021::JavaVM,
    f: impl FnOnce(&mut jni021::JNIEnv) -> jni021::errors::Result<T>,
) -> Result<T, String> {
    let mut env = vm
        .attach_current_thread_permanently()
        .map_err(|e| e.to_string())?;
    let r = env.with_local_frame(16, |env| f(env));
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    r.map_err(|e| e.to_string())
}

impl pm_out_local::bit_perfect::TrackBackend for JniBitPerfect {
    fn open(
        &self,
        dac_id: i32,
        entry: &pm_out_local::usb_dac::MixerEntry,
    ) -> Result<
        (
            Box<dyn pm_out_local::bit_perfect::Track>,
            pm_out_local::bit_perfect::OpenReport,
        ),
        pm_out_local::bit_perfect::OpenError,
    > {
        use jni021::objects::{JClass, JValue};
        let answer = with_jvm(&self.vm, |env| {
            let class: &JClass = self.class.as_obj().into();
            let s = env
                .call_static_method(
                    class,
                    "open",
                    "(IIII)Ljava/lang/String;",
                    &[
                        JValue::Int(dac_id),
                        JValue::Int(entry.encoding),
                        JValue::Int(entry.rate as i32),
                        JValue::Int(entry.channel_mask),
                    ],
                )?
                .l()?;
            Ok(String::from(env.get_string(&s.into())?))
        })
        .map_err(pm_out_local::bit_perfect::OpenError::Refused)?;
        let (handle, buffer_frames, report) = pm_out_local::bit_perfect::parse_open_answer(&answer)?;
        let track = JniTrack {
            vm: self.vm.clone(),
            class: self.class.clone(),
            handle,
            buffer_frames,
        };
        Ok((Box::new(track), report))
    }

    fn media_volume(&self) -> Option<u8> {
        use jni021::objects::JClass;
        let p = with_jvm(&self.vm, |env| {
            let class: &JClass = self.class.as_obj().into();
            env.call_static_method(class, "mediaVolume", "()I", &[])?.i()
        })
        .ok()?;
        u8::try_from(p).ok().filter(|p| *p <= 100)
    }

    fn media_db_on_dac(&self) -> Option<f32> {
        use jni021::objects::JClass;
        let db = with_jvm(&self.vm, |env| {
            let class: &JClass = self.class.as_obj().into();
            env.call_static_method(class, "mediaVolumeDbOnUsb", "()F", &[])?
                .f()
        })
        .ok()?;
        // NaN: Android can't say for the DAC; -inf: silent (both: the hand-over mutes)
        (!db.is_nan()).then_some(db)
    }
}

/// Exclusive USB's app side (`UsbExclusive.kt`) for pm-out-local's `exclusive`: Android's
/// permission for the DAC (asked only when the listener chose Exclusive USB), and the open device's
/// file descriptor, on which the app's driver speaks usbfs.
struct JniUsbExclusive {
    vm: std::sync::Arc<jni021::JavaVM>,
    class: jni021::objects::GlobalRef,
}

impl JniUsbExclusive {
    /// `UsbExclusive.open`'s (or `openDetached`'s) answer as the open device, or why not (with the
    /// device when a descriptor was handed over: it goes back and is closed).
    fn opened(answer: &str) -> Result<pm_out_local::exclusive::Opened, pm_out_local::exclusive::OpenRefused> {
        use pm_out_local::exclusive::{parse_open_answer, usbfs, OpenRefused};
        match parse_open_answer(answer) {
            Ok((fd, raw)) => Ok((usbfs(fd), raw)),
            Err((why, opened)) => Err(OpenRefused {
                why,
                partial: opened.map(|(fd, raw)| (usbfs(fd), raw)),
            }),
        }
    }

    fn ids(usb: pm_out_local::exclusive::UsbIds) -> [jni021::objects::JValue<'static, 'static>; 2] {
        [
            jni021::objects::JValue::Int(i32::from(usb.vendor_id)),
            jni021::objects::JValue::Int(i32::from(usb.product_id)),
        ]
    }
}

impl pm_out_local::exclusive::UsbBackend for JniUsbExclusive {
    fn has_permission(&self, usb: pm_out_local::exclusive::UsbIds) -> bool {
        use jni021::objects::JClass;
        with_jvm(&self.vm, |env| {
            let class: &JClass = self.class.as_obj().into();
            env.call_static_method(class, "hasPermission", "(II)Z", &Self::ids(usb))?
                .z()
        })
        .unwrap_or(false)
    }

    fn request_permission(&self, usb: pm_out_local::exclusive::UsbIds) {
        use jni021::objects::JClass;
        let r = with_jvm(&self.vm, |env| {
            let class: &JClass = self.class.as_obj().into();
            env.call_static_method(class, "requestPermission", "(II)V", &Self::ids(usb))?;
            Ok(())
        });
        if let Err(e) = r {
            tracing::warn!("Exclusive USB: cannot ask Android's permission: {e}");
        }
    }

    fn open(
        &self,
        usb: pm_out_local::exclusive::UsbIds,
    ) -> Result<pm_out_local::exclusive::Opened, pm_out_local::exclusive::OpenRefused> {
        use jni021::objects::JClass;
        use pm_out_local::exclusive::OpenRefused;
        let answer = with_jvm(&self.vm, |env| {
            let class: &JClass = self.class.as_obj().into();
            let s = env
                .call_static_method(class, "open", "(II)Ljava/lang/String;", &Self::ids(usb))?
                .l()?;
            Ok(String::from(env.get_string(&s.into())?))
        })
        .map_err(|why| OpenRefused { why, partial: None })?;
        Self::opened(&answer)
    }

    fn open_detached(
        &self,
        device_name: &str,
    ) -> Result<pm_out_local::exclusive::Opened, pm_out_local::exclusive::OpenRefused> {
        use jni021::objects::{JClass, JValue};
        use pm_out_local::exclusive::OpenRefused;
        let answer = with_jvm(&self.vm, |env| {
            let class: &JClass = self.class.as_obj().into();
            let name = env.new_string(device_name)?;
            let s = env
                .call_static_method(
                    class,
                    "openDetached",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                    &[JValue::Object(&name)],
                )?
                .l()?;
            Ok(String::from(env.get_string(&s.into())?))
        })
        .map_err(|why| OpenRefused { why, partial: None })?;
        Self::opened(&answer)
    }

    fn forget_claim(&self) {
        use jni021::objects::JClass;
        let r = with_jvm(&self.vm, |env| {
            let class: &JClass = self.class.as_obj().into();
            env.call_static_method(class, "forgetClaim", "()V", &[])?;
            Ok(())
        });
        if let Err(e) = r {
            tracing::warn!("Exclusive USB: forget the claim: {e}");
        }
    }

    fn held_identity(&self) -> Option<pm_out_local::exclusive::DacIdentity> {
        use jni021::objects::JClass;
        let answer = with_jvm(&self.vm, |env| {
            let class: &JClass = self.class.as_obj().into();
            let s = env
                .call_static_method(class, "heldIdentity", "()Ljava/lang/String;", &[])?
                .l()?;
            Ok(String::from(env.get_string(&s.into())?))
        });
        answer
            .ok()
            .and_then(|a| pm_out_local::exclusive::parse_identity(&a))
    }

    fn identity_of(
        &self,
        usb: pm_out_local::exclusive::UsbIds,
    ) -> Option<pm_out_local::exclusive::DacIdentity> {
        use jni021::objects::JClass;
        let answer = with_jvm(&self.vm, |env| {
            let class: &JClass = self.class.as_obj().into();
            let s = env
                .call_static_method(class, "identityOf", "(II)Ljava/lang/String;", &Self::ids(usb))?
                .l()?;
            Ok(String::from(env.get_string(&s.into())?))
        });
        answer
            .ok()
            .and_then(|a| pm_out_local::exclusive::parse_identity(&a))
    }

    fn take_detached(&self) -> Vec<pm_out_local::exclusive::DacIdentity> {
        use jni021::objects::JClass;
        let answer = with_jvm(&self.vm, |env| {
            let class: &JClass = self.class.as_obj().into();
            let s = env
                .call_static_method(class, "takeDetached", "()Ljava/lang/String;", &[])?
                .l()?;
            Ok(String::from(env.get_string(&s.into())?))
        });
        answer
            .ok()
            .map(|a| pm_out_local::exclusive::parse_detached_devices(&a))
            .unwrap_or_default()
    }

    fn claims(&self) -> pm_out_local::exclusive::Claims {
        use jni021::objects::JClass;
        let answer = with_jvm(&self.vm, |env| {
            let class: &JClass = self.class.as_obj().into();
            let s = env
                .call_static_method(class, "detached", "()Ljava/lang/String;", &[])?
                .l()?;
            Ok(String::from(env.get_string(&s.into())?))
        });
        answer
            .ok()
            .map(|a| pm_out_local::exclusive::parse_claims(&a))
            .unwrap_or_default()
    }

    fn release(&self) {
        use jni021::objects::JClass;
        let r = with_jvm(&self.vm, |env| {
            let class: &JClass = self.class.as_obj().into();
            env.call_static_method(class, "release", "()V", &[])?;
            Ok(())
        });
        if let Err(e) = r {
            tracing::warn!("Exclusive USB: release: {e}");
        }
    }
}

/// One `AudioTrack` of `BitPerfect.kt`, by its handle.
struct JniTrack {
    vm: std::sync::Arc<jni021::JavaVM>,
    class: jni021::objects::GlobalRef,
    handle: i64,
    buffer_frames: u64,
}

impl JniTrack {
    fn call(&self, method: &str) {
        use jni021::objects::{JClass, JValue};
        let r = with_jvm(&self.vm, |env| {
            let class: &JClass = self.class.as_obj().into();
            env.call_static_method(class, method, "(J)V", &[JValue::Long(self.handle)])?;
            Ok(())
        });
        if let Err(e) = r {
            tracing::warn!("bit-perfect player {method}: {e}");
        }
    }
}

/// `BitPerfect.GONE`.
const TRACK_GONE: i32 = -1000;

impl pm_out_local::bit_perfect::Track for JniTrack {
    fn write(&mut self, bytes: &[u8]) -> Result<(), pm_out_local::bit_perfect::TrackError> {
        use jni021::objects::{JClass, JValue};
        use pm_out_local::bit_perfect::TrackError;
        let mut at = 0;
        while at < bytes.len() {
            let rest = &bytes[at..];
            let n = with_jvm(&self.vm, |env| {
                // SAFETY: `rest` outlives the call, and Java only reads from the buffer.
                let buf = unsafe { env.new_direct_byte_buffer(rest.as_ptr() as *mut u8, rest.len())? };
                let class: &JClass = self.class.as_obj().into();
                env.call_static_method(
                    class,
                    "write",
                    "(JLjava/nio/ByteBuffer;I)I",
                    &[
                        JValue::Long(self.handle),
                        JValue::Object(&buf),
                        JValue::Int(rest.len() as i32),
                    ],
                )?
                .i()
            })
            .map_err(TrackError::Other)?;
            match n {
                TRACK_GONE => return Err(TrackError::Gone),
                n if n < 0 => return Err(TrackError::Other(format!("AudioTrack error {n}"))),
                0 => std::thread::sleep(std::time::Duration::from_millis(5)),
                n => at += n as usize,
            }
        }
        Ok(())
    }

    fn played_frames(&self) -> u64 {
        use jni021::objects::{JClass, JValue};
        with_jvm(&self.vm, |env| {
            let class: &JClass = self.class.as_obj().into();
            env.call_static_method(class, "played", "(J)J", &[JValue::Long(self.handle)])?
                .j()
        })
        .map(|n| n.max(0) as u64)
        .unwrap_or(0)
    }

    fn buffer_frames(&self) -> u64 {
        self.buffer_frames
    }

    fn pause(&mut self) {
        self.call("pause");
    }

    fn play(&mut self) {
        self.call("play");
    }

    fn flush(&mut self) {
        self.call("flush");
    }

    fn stop(&mut self) {
        self.call("stop");
    }

    fn still_confirmed(&self) -> Result<(), String> {
        use jni021::objects::{JClass, JValue};
        let why = with_jvm(&self.vm, |env| {
            let class: &JClass = self.class.as_obj().into();
            let s = env
                .call_static_method(
                    class,
                    "status",
                    "(J)Ljava/lang/String;",
                    &[JValue::Long(self.handle)],
                )?
                .l()?;
            Ok(String::from(env.get_string(&s.into())?))
        })?;
        if why.is_empty() {
            Ok(())
        } else {
            Err(why)
        }
    }
}

/// Waits for `UsbAudio.next` reports for as long as the app runs.
fn usb_audio_loop(vm: jni021::JavaVM, class: jni021::objects::GlobalRef, changed: impl Fn()) {
    use jni021::objects::{JClass, JValue};
    let mut env = match vm.attach_current_thread_as_daemon() {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("no USB DAC detection: {e}");
            return;
        }
    };
    let mut seen: i64 = 0;
    loop {
        let answer = env.with_local_frame(8, |env| -> jni021::errors::Result<String> {
            let class: &JClass = class.as_obj().into();
            let s = env
                .call_static_method(
                    class,
                    "next",
                    "(JJ)Ljava/lang/String;",
                    &[JValue::Long(seen), JValue::Long(30_000)],
                )?
                .l()?;
            Ok(env.get_string(&s.into())?.into())
        });
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        let answer = match answer {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!("USB DAC detection: {e}");
                std::thread::sleep(std::time::Duration::from_secs(5));
                continue;
            }
        };
        let Some((version, report)) = pm_out_local::usb_dac::parse_poll(&answer) else {
            continue;
        };
        let version = i64::try_from(version).unwrap_or(i64::MAX);
        if version == seen {
            continue;
        }
        seen = version;
        // A report that can't be read clears the DAC rather than keeping the last one
        if pm_out_local::usb_dac::apply_report(report) {
            changed();
        }
    }
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
