#![cfg(target_os = "android")]

//! Storage Access Framework bridge: ask the Java side (`SafBridge`) for files, which copies the
//! picked documents into the app's own directory and hands back real paths — the engine's file
//! services work with paths, not `content://` URIs.
//!
//! Everything here runs on a background thread: the pickers are posted to the UI thread by Java and
//! this side polls the `isDone` / `isSaveDone` flags, so the egui UI thread never blocks. Import
//! results travel through the filesystem (`<filesDir>/import-manifest.txt`, written by Java) rather
//! than JNI marshalling, so the JNI surface stays at a handful of calls.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use jni::objects::{JObject, JValue};
use jni::{JavaVM, jni_sig, jni_str};

/// The manifest the Java side writes; one picked path per line.
const MANIFEST: &str = "import-manifest.txt";

/// Attach the current thread, look up the activity object and run `body`.
#[allow(unsafe_code)]
fn call<T>(activity: usize, body: impl FnOnce(&mut jni::Env<'_>, &JObject<'_>) -> jni::errors::Result<T>) -> Option<T> {
    let ctx = ndk_context::android_context();
    // SAFETY: `ndk_context` is set up by android-activity before `android_main` runs and stays
    // valid for the life of the process; `from_raw` only wraps the pointer in the jni crate's type
    // (it caches the JVM, which the JVM itself ref-counts for the process lifetime).
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
    match vm.attach_current_thread(|env| -> jni::errors::Result<T> {
        // SAFETY: the pointer is `AndroidApp::activity_as_ptr()`, a reference to the running
        // activity that is valid for the whole process; `JObject::from_raw` borrows it for this JNI
        // frame.
        let activity = unsafe { JObject::from_raw(env, activity as *mut _) };
        body(env, &activity)
    }) {
        Ok(value) => Some(value),
        Err(err) => {
            /* Android-port: this was `.ok()`, which turned every JNI failure into `None`. That made
             * "the picker never appeared" indistinguishable from "the user cancelled" -- the wrong
             * default for a file dialog, and the reason this bug stayed hidden. Log it: a missing
             * method or a thrown exception is a bug, silence is not. */
            log::error!("SAF: JNI call failed: {err:?}");
            None
        }
    }
}

/// Open the system document picker and return the paths copied into the app directory (empty when
/// the user cancels, or on any failure — the caller just does nothing).
///
/// `activity` is `AndroidApp::activity_as_ptr()` (the Java activity object; `ndk_context` hands out
/// the *Application* instead, which has no `startActivityForResult`), and `data_dir` is
/// `AndroidApp::internal_data_path()`, the same `filesDir` the Java side uses.
#[allow(unsafe_code)]
pub fn pick_media(activity: usize, data_dir: &Path, timeout: Duration) -> Vec<String> {
    let manifest: PathBuf = data_dir.join(MANIFEST);
    let _ = std::fs::remove_file(&manifest);

    let picked = call(activity, |env, activity| {
        env.call_method(activity, jni_str!("safPickMedia"), jni_sig!("()V"), &[])?;
        let deadline = Instant::now() + timeout;
        loop {
            let done = env.call_method(activity, jni_str!("safIsDone"), jni_sig!("()Z"), &[])?.z().unwrap_or(false);
            if done {
                return Ok(true);
            }
            if Instant::now() > deadline {
                log::warn!("SAF: picker timed out after {timeout:?}");
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });

    match picked {
        Some(true) => read_manifest(&manifest),
        _ => Vec::new(),
    }
}

/// Ask the user where an export should land (`ACTION_CREATE_DOCUMENT`); true when a destination was
/// chosen. The URI stays on the Java side until [`publish`] spends it.
#[allow(unsafe_code)]
pub fn pick_save(activity: usize, title: &str, timeout: Duration) -> bool {
    let result = call(activity, |env, activity| {
        let title = env.new_string(title)?;
        env.call_method(activity, jni_str!("safPickSave"), jni_sig!("(Ljava/lang/String;)V"), &[JValue::Object(&title)])?;
        let deadline = Instant::now() + timeout;
        loop {
            let done = env.call_method(activity, jni_str!("safSaveDone"), jni_sig!("()Z"), &[])?.z().unwrap_or(false);
            if done {
                return env.call_method(activity, jni_str!("safHasSaveUri"), jni_sig!("()Z"), &[])?.z();
            }
            if Instant::now() > deadline {
                log::warn!("SAF: save picker timed out after {timeout:?}");
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
    result.unwrap_or(false)
}

/// Copy a finished file to the destination chosen by [`pick_save`].
#[allow(unsafe_code)]
pub fn publish(activity: usize, path: &str) -> bool {
    let result = call(activity, |env, activity| {
        let path = env.new_string(path)?;
        env.call_method(activity, jni_str!("safPublish"), jni_sig!("(Ljava/lang/String;)Z"), &[JValue::Object(&path)])?.z()
    });
    result.unwrap_or(false)
}

fn read_manifest(path: &Path) -> Vec<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let paths: Vec<String> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
            log::info!("SAF: {} file(s) picked", paths.len());
            paths
        }
        Err(e) => {
            log::info!("SAF: nothing picked ({e})");
            Vec::new()
        }
    }
}
