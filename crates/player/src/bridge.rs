//! The seam between the Rust core and the phone's native side: Kotlin on Android
//! (`com.limusic.mobile.LimusicBridge`, over JNI), Swift on iOS (`LimusicBridge.swift`, over a
//! C ABI pair).
//!
//! Commands go out as one JSON object per call. The native side only posts the work to its main
//! thread and returns, so a call never blocks on the UI thread and never deadlocks a caller that
//! is itself on it (Tauri runs sync commands there). Events come back tagged with a `topic` that
//! picks the handler.
//!
//! The native side connects at process start (`Application.onCreate`, `main.mm`), normally long
//! before the app's setup sends anything. Commands sent before that are queued and flushed on
//! connect, in order.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::{Map, Value};

/// Commands sent before the native side connected. `None` once it has.
static EARLY: Mutex<Option<Vec<String>>> = Mutex::new(Some(Vec::new()));

type Handler = Box<dyn Fn(&Value) + Send + Sync>;
static HANDLERS: OnceLock<Mutex<HashMap<String, Handler>>> = OnceLock::new();

fn handlers() -> &'static Mutex<HashMap<String, Handler>> {
    HANDLERS.get_or_init(Default::default)
}

/// Route events tagged `topic` to `f`. One handler per topic; a second call replaces the first.
///
/// The handler runs on the native main thread, inside the native call: it must hand work off
/// (a channel send, a spawned task) rather than do it.
pub fn on(topic: &str, f: impl Fn(&Value) + Send + Sync + 'static) {
    handlers().lock().unwrap().insert(topic.to_owned(), Box::new(f));
}

/// Send `cmd` with `args` (an object, or `Value::Null` for none) to the native side.
pub fn send(cmd: &str, args: Value) {
    let mut obj = match args {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    obj.insert("cmd".into(), Value::String(cmd.to_owned()));
    let json = Value::Object(obj).to_string();
    {
        let mut early = EARLY.lock().unwrap();
        if let Some(queue) = early.as_mut() {
            queue.push(json);
            return;
        }
    }
    transport::command(&json);
}

/// The native side is up: hand it everything sent so far.
fn connected() {
    let queued = EARLY.lock().unwrap().take().unwrap_or_default();
    for json in queued {
        transport::command(&json);
    }
}

/// One event from the native side, as JSON text.
fn deliver(text: &str) {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        tracing::warn!("bridge: unparseable event from the native side");
        return;
    };
    let topic = value.get("topic").and_then(Value::as_str).unwrap_or_default();
    if let Some(h) = handlers().lock().unwrap().get(topic) {
        h(&value);
    }
}

#[cfg(target_os = "android")]
mod transport {
    use std::sync::OnceLock;

    use jni::objects::{GlobalRef, JClass, JString, JValue};
    use jni::{JNIEnv, JavaVM};

    static VM: OnceLock<JavaVM> = OnceLock::new();
    static CLASS: OnceLock<GlobalRef> = OnceLock::new();

    pub(super) fn command(json: &str) {
        let (Some(vm), Some(class)) = (VM.get(), CLASS.get()) else { return };
        let mut env = match vm.attach_current_thread_permanently() {
            Ok(env) => env,
            Err(e) => {
                tracing::warn!(error = %e, "bridge: could not attach this thread to the JVM");
                return;
            }
        };
        let res = (|| -> jni::errors::Result<()> {
            let arg = env.new_string(json)?;
            let class: &JClass = class.as_obj().into();
            env.call_static_method(
                class,
                "command",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&arg)],
            )?;
            env.delete_local_ref(arg)?;
            Ok(())
        })();
        if res.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        if let Err(e) = res {
            tracing::warn!(error = %e, "bridge: command failed");
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_com_limusic_mobile_LimusicBridge_nativeInit(
        env: JNIEnv,
        class: JClass,
    ) {
        let Ok(vm) = env.get_java_vm() else { return };
        let Ok(global) = env.new_global_ref(&class) else { return };
        let _ = VM.set(vm);
        let _ = CLASS.set(global);
        super::connected();
    }

    #[no_mangle]
    pub extern "system" fn Java_com_limusic_mobile_LimusicBridge_nativeEvent(
        mut env: JNIEnv,
        _class: JClass,
        json: JString,
    ) {
        let Ok(text) = env.get_string(&json).map(String::from) else { return };
        super::deliver(&text);
    }
}

#[cfg(target_os = "ios")]
mod transport {
    use std::ffi::{c_char, CStr, CString};
    use std::sync::OnceLock;

    type Receiver = extern "C" fn(*const c_char);
    static RECEIVER: OnceLock<Receiver> = OnceLock::new();

    pub(super) fn command(json: &str) {
        let Some(receiver) = RECEIVER.get() else { return };
        let Ok(text) = CString::new(json) else { return };
        receiver(text.as_ptr());
    }

    /// Swift hands over the function that takes commands. Called once, before `start_app`.
    #[no_mangle]
    pub extern "C" fn limusic_bridge_connect(receiver: Receiver) {
        let _ = RECEIVER.set(receiver);
        super::connected();
    }

    /// One event from Swift: a NUL-terminated UTF-8 JSON object, borrowed for the call.
    ///
    /// # Safety
    /// `json` must be null or point to a valid NUL-terminated string.
    #[no_mangle]
    pub unsafe extern "C" fn limusic_bridge_event(json: *const c_char) {
        if json.is_null() {
            return;
        }
        super::deliver(&CStr::from_ptr(json).to_string_lossy());
    }
}
