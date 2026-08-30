use crate::subscription::{SubscriptionConfig, SubscriptionState};
use std::path::{Path, PathBuf};

const MAX_SERVER_BYTES: usize = 4 * 1024;
const MAX_USERNAME_BYTES: usize = 1024;
const MAX_PASSWORD_BYTES: usize = 8 * 1024;
const MAX_TOPIC_BYTES: usize = 64;
const MAX_CONFIG_BYTES: usize = 16 * 1024;

const INVALID_SESSION: &str = "INVALID_SESSION";
const STALE_SESSION: &str = "STALE_SESSION";
const INVALID_DATA_DIR: &str = "INVALID_DATA_DIR";
const DATA_DIR_CHANGED: &str = "DATA_DIR_CHANGED";
const INVALID_CONFIG: &str = "INVALID_CONFIG";

type NativeResult<T> = Result<T, &'static str>;

/// JNI-facing connection fields. Deliberately does not implement `Debug`: this
/// structure owns the clear-text password briefly while a subscription is
/// configured and must never be included in diagnostic output.
struct NativeConfigFields {
    server: String,
    username: String,
    password: String,
    topic: String,
    allow_insecure_http: bool,
}

impl NativeConfigFields {
    fn into_subscription_config(self) -> NativeResult<SubscriptionConfig> {
        validate_text(&self.server, MAX_SERVER_BYTES)?;
        validate_text(&self.username, MAX_USERNAME_BYTES)?;
        validate_text(&self.password, MAX_PASSWORD_BYTES)?;
        validate_text(&self.topic, MAX_TOPIC_BYTES)?;

        let total = self
            .server
            .len()
            .checked_add(self.username.len())
            .and_then(|size| size.checked_add(self.password.len()))
            .and_then(|size| size.checked_add(self.topic.len()))
            .ok_or(INVALID_CONFIG)?;
        if total > MAX_CONFIG_BYTES {
            return Err(INVALID_CONFIG);
        }

        Ok(SubscriptionConfig {
            server: self.server,
            username: self.username,
            password: self.password,
            topic: self.topic,
            allow_insecure_http: self.allow_insecure_http,
        })
    }
}

fn validate_text(value: &str, maximum: usize) -> NativeResult<()> {
    if value.len() > maximum || value.contains('\0') {
        Err(INVALID_CONFIG)
    } else {
        Ok(())
    }
}

fn normalize_data_dir(path: PathBuf) -> NativeResult<PathBuf> {
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return Err(INVALID_DATA_DIR);
    }
    let normalized = std::fs::canonicalize(path).map_err(|_| INVALID_DATA_DIR)?;
    if !normalized.is_dir() {
        return Err(INVALID_DATA_DIR);
    }
    Ok(normalized)
}

#[derive(Default)]
struct EngineControl {
    data_dir: Option<PathBuf>,
    last_session: Option<i64>,
}

impl EngineControl {
    fn accept_session(&mut self, session: i64) -> NativeResult<()> {
        if session <= 0 {
            return Err(INVALID_SESSION);
        }
        if self
            .last_session
            .is_some_and(|last_session| session < last_session)
        {
            return Err(STALE_SESSION);
        }
        self.last_session = Some(session);
        Ok(())
    }

    /// Pins the first canonical application data directory for the process.
    /// Returns true exactly once so the caller can initialize `appdata` without
    /// permitting later JNI calls to redirect SQLite/rules storage.
    fn pin_data_dir(&mut self, data_dir: &Path) -> NativeResult<bool> {
        match self.data_dir.as_deref() {
            Some(existing) if existing == data_dir => Ok(false),
            Some(_) => Err(DATA_DIR_CHANGED),
            None => {
                self.data_dir = Some(data_dir.to_path_buf());
                Ok(true)
            }
        }
    }
}

#[cfg(target_os = "android")]
mod platform {
    use super::*;
    use crate::subscription::{
        SubscriptionController, SubscriptionCore, SubscriptionMessage, SubscriptionSink,
    };
    use jni::objects::{GlobalRef, JObject, JString, JValue};
    use jni::sys::{jboolean, jlong, JNI_FALSE, JNI_TRUE};
    use jni::{JNIEnv, JavaVM};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use tokio::runtime::{Builder, Runtime};

    const ENGINE_INIT_FAILED: &str = "ENGINE_INIT_FAILED";
    const ENGINE_LOCK_FAILED: &str = "ENGINE_LOCK_FAILED";
    const CALLBACK_INVALID: &str = "CALLBACK_INVALID";
    const CALLBACK_FAILED: &str = "CALLBACK_FAILED";
    const JNI_STRING_FAILED: &str = "JNI_STRING_FAILED";
    const JNI_BOOLEAN_INVALID: &str = "JNI_BOOLEAN_INVALID";
    const JNI_PANIC: &str = "JNI_PANIC";
    const MAX_DATA_DIR_BYTES: usize = 4 * 1024;

    static ENGINE: OnceLock<Result<AndroidEngine, &'static str>> = OnceLock::new();

    struct CallbackTarget {
        vm: JavaVM,
        callback: GlobalRef,
    }

    impl CallbackTarget {
        fn new(env: &JNIEnv<'_>, callback: &JObject<'_>) -> NativeResult<Self> {
            if callback.as_raw().is_null() {
                return Err(CALLBACK_INVALID);
            }
            let vm = env.get_java_vm().map_err(|_| CALLBACK_INVALID)?;
            let callback = env.new_global_ref(callback).map_err(|_| CALLBACK_INVALID)?;
            Ok(Self { vm, callback })
        }

        fn state_changed(&self, session: i64, state: SubscriptionState) -> NativeResult<()> {
            let mut env = self
                .vm
                .attach_current_thread()
                .map_err(|_| CALLBACK_FAILED)?;
            let state = JObject::from(
                env.new_string(state.as_str())
                    .map_err(|_| callback_failure(&env))?,
            );
            let result = env.call_method(
                self.callback.as_obj(),
                "onNativeState",
                "(JLjava/lang/String;)V",
                &[JValue::Long(session as jlong), JValue::Object(&state)],
            );
            finish_callback(&env, result.map(|_| ()))
        }

        fn message_received(
            &self,
            session: i64,
            title: &str,
            message: &str,
            otp: Option<&str>,
        ) -> NativeResult<()> {
            let mut env = self
                .vm
                .attach_current_thread()
                .map_err(|_| CALLBACK_FAILED)?;
            let title = JObject::from(env.new_string(title).map_err(|_| callback_failure(&env))?);
            let message = JObject::from(
                env.new_string(message)
                    .map_err(|_| callback_failure(&env))?,
            );
            let otp = match otp {
                Some(value) => {
                    JObject::from(env.new_string(value).map_err(|_| callback_failure(&env))?)
                }
                None => JObject::null(),
            };
            let result = env.call_method(
                self.callback.as_obj(),
                "onNativeMessage",
                "(JLjava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
                &[
                    JValue::Long(session as jlong),
                    JValue::Object(&title),
                    JValue::Object(&message),
                    JValue::Object(&otp),
                ],
            );
            finish_callback(&env, result.map(|_| ()))
        }
    }

    /// Each sink has an internal epoch in addition to the Kotlin session token.
    /// This suppresses the controller's forced `stopped` event from an old sink
    /// even when a caller idempotently reuses the same session number.
    struct AndroidSink {
        callback: Arc<CallbackTarget>,
        session: i64,
        epoch: u64,
        current_epoch: Arc<AtomicU64>,
    }

    impl AndroidSink {
        fn is_current(&self) -> bool {
            self.current_epoch.load(Ordering::SeqCst) == self.epoch
        }
    }

    impl SubscriptionSink for AndroidSink {
        fn state_changed(&self, state: SubscriptionState) {
            if !self.is_current() {
                return;
            }
            if let Err(code) = self.callback.state_changed(self.session, state) {
                log_error(code);
            }
        }

        fn message_received(&self, message: SubscriptionMessage) -> Result<(), String> {
            // The core invokes the sink only after the message and cursor commit in one
            // transaction. A concurrent reconfiguration may already have advanced the epoch,
            // but suppressing this callback would permanently lose the notification because
            // the replacement stream resumes after the committed cursor. Kotlin separately
            // binds the callback object to the lifetime of its Service instance.
            let rules = crate::rules::load();
            let otp = crate::rules::find_otp(&message.message, &rules);
            self.callback
                .message_received(
                    self.session,
                    &message.title,
                    &message.message,
                    otp.as_deref(),
                )
                .map_err(str::to_owned)
        }
    }

    struct AndroidEngine {
        runtime: Runtime,
        controller: SubscriptionController,
        core: SubscriptionCore,
        operation: Mutex<EngineControl>,
        current_epoch: Arc<AtomicU64>,
    }

    impl AndroidEngine {
        fn new() -> NativeResult<Self> {
            let runtime = Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("ntfy-subscriber")
                .enable_all()
                .build()
                .map_err(|_| ENGINE_INIT_FAILED)?;
            Ok(Self {
                runtime,
                controller: SubscriptionController::default(),
                core: SubscriptionCore::default(),
                operation: Mutex::new(EngineControl::default()),
                current_epoch: Arc::new(AtomicU64::new(0)),
            })
        }

        fn configure(
            &self,
            data_dir: NativeResult<PathBuf>,
            session: i64,
            config: NativeResult<SubscriptionConfig>,
            callback: Arc<CallbackTarget>,
        ) -> NativeResult<()> {
            let mut operation = self.operation.lock().map_err(|_| ENGINE_LOCK_FAILED)?;
            operation.accept_session(session)?;

            let epoch = self
                .current_epoch
                .fetch_add(1, Ordering::SeqCst)
                .wrapping_add(1);

            let data_dir = match data_dir {
                Ok(data_dir) => data_dir,
                Err(code) => {
                    self.controller.stop();
                    drop(operation);
                    report_configuration_error(&callback, session);
                    return Err(code);
                }
            };
            let initialize_appdata = match operation.pin_data_dir(&data_dir) {
                Ok(initialize) => initialize,
                Err(code) => {
                    self.controller.stop();
                    drop(operation);
                    report_configuration_error(&callback, session);
                    return Err(code);
                }
            };
            if initialize_appdata {
                crate::appdata::set(data_dir);
            }

            let config = match config.and_then(validate_endpoint) {
                Ok(config) => config,
                Err(code) => {
                    self.controller.stop();
                    drop(operation);
                    report_configuration_error(&callback, session);
                    return Err(code);
                }
            };

            let sink = AndroidSink {
                callback,
                session,
                epoch,
                current_epoch: Arc::clone(&self.current_epoch),
            };
            let handle = self.runtime.handle().clone();
            self.controller
                .reconfigure(self.core.clone(), config, sink, move |task| {
                    std::mem::drop(handle.spawn(task));
                });
            Ok(())
        }

        fn stop(&self) -> NativeResult<()> {
            let _operation = self.operation.lock().map_err(|_| ENGINE_LOCK_FAILED)?;
            self.controller.stop();
            Ok(())
        }
    }

    fn validate_endpoint(config: SubscriptionConfig) -> NativeResult<SubscriptionConfig> {
        crate::endpoint::validate_subscription_endpoint(
            &config.server,
            &config.topic,
            &config.username,
            &config.password,
            config.allow_insecure_http,
        )
        .map_err(|_| INVALID_CONFIG)?;
        Ok(config)
    }

    fn engine() -> NativeResult<&'static AndroidEngine> {
        ENGINE
            .get_or_init(AndroidEngine::new)
            .as_ref()
            .map_err(|code| *code)
    }

    fn callback_failure(env: &JNIEnv<'_>) -> &'static str {
        clear_pending_exception(env);
        CALLBACK_FAILED
    }

    fn finish_callback<T>(env: &JNIEnv<'_>, result: jni::errors::Result<T>) -> NativeResult<()> {
        if result.is_err() {
            return Err(callback_failure(env));
        }
        match env.exception_check() {
            Ok(false) => Ok(()),
            Ok(true) => {
                clear_pending_exception(env);
                Err(CALLBACK_FAILED)
            }
            Err(_) => Err(CALLBACK_FAILED),
        }
    }

    fn clear_pending_exception(env: &JNIEnv<'_>) {
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
    }

    fn report_configuration_error(callback: &CallbackTarget, session: i64) {
        if let Err(code) = callback.state_changed(session, SubscriptionState::ConfigurationError) {
            log_error(code);
        }
    }

    fn java_string(env: &mut JNIEnv<'_>, value: &JString<'_>) -> NativeResult<String> {
        env.get_string(value)
            .map(Into::into)
            .map_err(|_| JNI_STRING_FAILED)
    }

    fn java_boolean(value: jboolean) -> NativeResult<bool> {
        match value {
            JNI_FALSE => Ok(false),
            JNI_TRUE => Ok(true),
            _ => Err(JNI_BOOLEAN_INVALID),
        }
    }

    struct JniConfigureArgs<'local> {
        data_dir: JString<'local>,
        session: jlong,
        server: JString<'local>,
        username: JString<'local>,
        password: JString<'local>,
        topic: JString<'local>,
        allow_insecure_http: jboolean,
        callback: JObject<'local>,
    }

    fn configure_from_jni(env: &mut JNIEnv<'_>, args: JniConfigureArgs<'_>) -> jboolean {
        let callback = match CallbackTarget::new(env, &args.callback) {
            Ok(callback) => Arc::new(callback),
            Err(code) => {
                clear_pending_exception(env);
                log_error(code);
                return JNI_FALSE;
            }
        };

        let data_dir = java_string(env, &args.data_dir)
            .and_then(|value| {
                if value.len() > MAX_DATA_DIR_BYTES || value.contains('\0') {
                    Err(INVALID_DATA_DIR)
                } else {
                    Ok(PathBuf::from(value))
                }
            })
            .and_then(normalize_data_dir);
        let config = (|| {
            let fields = NativeConfigFields {
                server: java_string(env, &args.server)?,
                username: java_string(env, &args.username)?,
                password: java_string(env, &args.password)?,
                topic: java_string(env, &args.topic)?,
                allow_insecure_http: java_boolean(args.allow_insecure_http)?,
            };
            fields.into_subscription_config()
        })();
        clear_pending_exception(env);

        let result = engine().and_then(|engine| {
            engine.configure(data_dir, args.session, config, Arc::clone(&callback))
        });
        match result {
            Ok(()) => JNI_TRUE,
            Err(code) => {
                // Engine-level configuration failures report their state after
                // releasing the operation lock. Failures before engine entry
                // still need the same explicit state callback.
                if matches!(code, INVALID_SESSION | STALE_SESSION | ENGINE_INIT_FAILED) {
                    report_configuration_error(&callback, args.session);
                }
                log_error(code);
                JNI_FALSE
            }
        }
    }

    fn stop_from_jni() {
        let Some(engine) = ENGINE.get() else {
            return;
        };
        match engine {
            Ok(engine) => {
                if let Err(code) = engine.stop() {
                    log_error(code);
                }
            }
            Err(code) => log_error(code),
        }
    }

    fn log_error(code: &'static str) {
        // Codes are fixed literals. Never interpolate JNI configuration,
        // notification content, passwords, or OTPs into Android logs.
        eprintln!("[android-subscriber] {code}");
    }

    #[allow(non_snake_case)]
    #[no_mangle]
    pub extern "system" fn Java_app_ntfy_notifier_NativeSubscriber_nativeStart(
        mut env: JNIEnv<'_>,
        _this: JObject<'_>,
        data_dir: JString<'_>,
        session: jlong,
        server: JString<'_>,
        username: JString<'_>,
        password: JString<'_>,
        topic: JString<'_>,
        allow_insecure_http: jboolean,
        callback: JObject<'_>,
    ) -> jboolean {
        match catch_unwind(AssertUnwindSafe(|| {
            configure_from_jni(
                &mut env,
                JniConfigureArgs {
                    data_dir,
                    session,
                    server,
                    username,
                    password,
                    topic,
                    allow_insecure_http,
                    callback,
                },
            )
        })) {
            Ok(result) => result,
            Err(_) => {
                clear_pending_exception(&env);
                log_error(JNI_PANIC);
                JNI_FALSE
            }
        }
    }

    #[allow(non_snake_case)]
    #[no_mangle]
    pub extern "system" fn Java_app_ntfy_notifier_NativeSubscriber_nativeReconfigure(
        mut env: JNIEnv<'_>,
        _this: JObject<'_>,
        data_dir: JString<'_>,
        session: jlong,
        server: JString<'_>,
        username: JString<'_>,
        password: JString<'_>,
        topic: JString<'_>,
        allow_insecure_http: jboolean,
        callback: JObject<'_>,
    ) -> jboolean {
        match catch_unwind(AssertUnwindSafe(|| {
            configure_from_jni(
                &mut env,
                JniConfigureArgs {
                    data_dir,
                    session,
                    server,
                    username,
                    password,
                    topic,
                    allow_insecure_http,
                    callback,
                },
            )
        })) {
            Ok(result) => result,
            Err(_) => {
                clear_pending_exception(&env);
                log_error(JNI_PANIC);
                JNI_FALSE
            }
        }
    }

    #[allow(non_snake_case)]
    #[no_mangle]
    pub extern "system" fn Java_app_ntfy_notifier_NativeSubscriber_nativeStop(
        env: JNIEnv<'_>,
        _this: JObject<'_>,
    ) {
        if catch_unwind(AssertUnwindSafe(stop_from_jni)).is_err() {
            clear_pending_exception(&env);
            log_error(JNI_PANIC);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> NativeConfigFields {
        NativeConfigFields {
            server: "https://ntfy.example.com".to_string(),
            username: "user".to_string(),
            password: "secret".to_string(),
            topic: "alerts".to_string(),
            allow_insecure_http: false,
        }
    }

    #[test]
    fn state_names_match_android_callback_contract() {
        assert_eq!(SubscriptionState::Connecting.as_str(), "connecting");
        assert_eq!(SubscriptionState::Connected.as_str(), "connected");
        assert_eq!(SubscriptionState::Retrying.as_str(), "retrying");
        assert_eq!(
            SubscriptionState::ConfigurationError.as_str(),
            "configuration_error"
        );
        assert_eq!(SubscriptionState::Stopped.as_str(), "stopped");
    }

    #[test]
    fn native_config_fields_preserve_the_connection_contract() {
        let config = fields().into_subscription_config().unwrap();
        assert_eq!(config.server, "https://ntfy.example.com");
        assert_eq!(config.username, "user");
        assert_eq!(config.password, "secret");
        assert_eq!(config.topic, "alerts");
        assert!(!config.allow_insecure_http);
    }

    #[test]
    fn native_config_rejects_oversize_and_nul_fields() {
        let mut oversized = fields();
        oversized.password = "p".repeat(MAX_PASSWORD_BYTES + 1);
        assert_eq!(
            oversized.into_subscription_config().unwrap_err(),
            INVALID_CONFIG
        );

        let mut nul = fields();
        nul.password = "before\0after".to_string();
        assert_eq!(nul.into_subscription_config().unwrap_err(), INVALID_CONFIG);
    }

    #[test]
    fn engine_control_pins_directory_and_orders_sessions() {
        let first = PathBuf::from("/data/user/0/app.ntfy.notifier/files");
        let second = PathBuf::from("/data/user/0/app.ntfy.notifier/other");
        let mut control = EngineControl::default();

        assert_eq!(control.accept_session(0), Err(INVALID_SESSION));
        assert_eq!(control.accept_session(10), Ok(()));
        assert_eq!(control.accept_session(9), Err(STALE_SESSION));
        assert_eq!(control.accept_session(10), Ok(()));
        assert_eq!(control.accept_session(11), Ok(()));
        assert_eq!(control.last_session, Some(11));

        assert_eq!(control.pin_data_dir(&first), Ok(true));
        assert_eq!(control.pin_data_dir(&first), Ok(false));
        assert_eq!(control.pin_data_dir(&second), Err(DATA_DIR_CHANGED));
        assert_eq!(control.data_dir.as_deref(), Some(first.as_path()));
    }

    #[test]
    fn normalize_data_dir_requires_an_existing_absolute_directory() {
        assert_eq!(
            normalize_data_dir(PathBuf::from("relative")),
            Err(INVALID_DATA_DIR)
        );
        let directory =
            std::env::temp_dir().join(format!("ntfy-android-subscriber-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let normalized = normalize_data_dir(directory.clone()).unwrap();
        assert!(normalized.is_absolute());
        assert!(normalized.is_dir());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
