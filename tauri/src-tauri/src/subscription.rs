use crate::config::Config;
use crate::endpoint::{self, ValidatedEndpoint};
use crate::sse::{DecodeOutcome, Decoder as SseDecoder, MAX_EVENT_BYTES};
use futures_util::future::BoxFuture;
use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::pin::Pin;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

const MAX_NTFY_JSON_BYTES: usize = MAX_EVENT_BYTES;
// ntfy enforces a 1 KiB title ceiling. The body cap is four times its default
// 4 KiB message limit so common self-hosted and base64 UnifiedPush payloads
// remain compatible while history growth stays bounded.
pub(crate) const MAX_TITLE_BYTES: usize = 1024;
pub(crate) const MAX_MESSAGE_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SubscriptionConfig {
    pub server: String,
    pub username: String,
    pub password: String,
    pub topic: String,
    pub allow_insecure_http: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SubscriptionKey {
    pub server: String,
    pub topic: String,
}

impl From<&Config> for SubscriptionConfig {
    fn from(config: &Config) -> Self {
        Self {
            server: config.server.clone(),
            username: config.username.clone(),
            password: config.password.clone(),
            topic: config.topic.clone(),
            allow_insecure_http: config.allow_insecure_http,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ValidatedSubscriptionConfig {
    endpoint: ValidatedEndpoint,
    key: SubscriptionKey,
    username: String,
    password: String,
    allow_insecure_http: bool,
}

impl TryFrom<SubscriptionConfig> for ValidatedSubscriptionConfig {
    type Error = String;

    fn try_from(config: SubscriptionConfig) -> Result<Self, Self::Error> {
        let endpoint = endpoint::validate_subscription_endpoint(
            &config.server,
            &config.topic,
            &config.username,
            &config.password,
            config.allow_insecure_http,
        )?;
        let mut key_url = endpoint.server.clone();
        if let Ok(mut segments) = key_url.path_segments_mut() {
            segments.pop_if_empty();
        }
        let key = SubscriptionKey {
            server: key_url.to_string(),
            topic: config.topic,
        };
        Ok(Self {
            endpoint,
            key,
            username: config.username,
            password: config.password,
            allow_insecure_http: config.allow_insecure_http,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SubscriptionState {
    Connecting,
    Connected,
    Retrying,
    ConfigurationError,
    Stopped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SubscriptionMessage {
    pub id: String,
    pub topic: String,
    pub title: String,
    pub message: String,
}

pub(crate) fn validate_persistable_message(
    message: &SubscriptionMessage,
) -> Result<(), &'static str> {
    validate_persistable_text(&message.title, &message.message)
}

pub(crate) fn validate_persistable_text(title: &str, message: &str) -> Result<(), &'static str> {
    if title.len() > MAX_TITLE_BYTES {
        return Err("消息标题超过 1024 字节上限");
    }
    if message.len() > MAX_MESSAGE_BYTES {
        return Err("消息正文超过 16384 字节上限");
    }
    if title.contains('\0') || message.contains('\0') {
        return Err("消息标题或正文包含 NUL 字符");
    }
    Ok(())
}

pub(crate) trait SubscriptionSink: Send + Sync {
    fn state_changed(&self, state: SubscriptionState);
    fn message_received(&self, message: SubscriptionMessage) -> Result<(), String>;
}

#[derive(Clone)]
pub(crate) struct GenerationGuard {
    generation: u64,
    current_generation: Arc<AtomicU64>,
    cancellation: CancellationToken,
}

impl GenerationGuard {
    pub(crate) fn is_current(&self) -> bool {
        !self.cancellation.is_cancelled()
            && self.current_generation.load(Ordering::SeqCst) == self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoreCommit {
    Inserted,
    Duplicate,
    StaleGeneration,
}

pub(crate) trait SubscriptionStore: Send + Sync {
    fn load_cursor(&self, key: &SubscriptionKey) -> Result<Option<String>, String>;

    fn commit_message(
        &self,
        key: &SubscriptionKey,
        message: &SubscriptionMessage,
        generation: &GenerationGuard,
    ) -> Result<StoreCommit, String>;
}

type SubscriptionStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, String>> + Send>>;

pub(crate) trait SubscriptionSource: Send + Sync {
    fn connect(
        &self,
        config: ValidatedSubscriptionConfig,
        since: Option<String>,
    ) -> BoxFuture<'static, Result<SubscriptionStream, String>>;
}

struct ReqwestSource {
    secure_client: Result<reqwest::Client, String>,
    insecure_client: Result<reqwest::Client, String>,
    loopback_secure_client: Result<reqwest::Client, String>,
    loopback_insecure_client: Result<reqwest::Client, String>,
}

impl Default for ReqwestSource {
    fn default() -> Self {
        Self {
            secure_client: build_client(false, false),
            insecure_client: build_client(true, false),
            loopback_secure_client: build_client(false, true),
            loopback_insecure_client: build_client(true, true),
        }
    }
}

fn build_client(
    allow_insecure_http: bool,
    bypass_system_proxy: bool,
) -> Result<reqwest::Client, String> {
    let redirect_policy = reqwest::redirect::Policy::custom(move |attempt| {
        match endpoint::validate_redirect(attempt.previous(), attempt.url(), allow_insecure_http) {
            Ok(()) => attempt.follow(),
            Err(error) => attempt.error(error),
        }
    });
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .redirect(redirect_policy);
    if bypass_system_proxy {
        builder = builder.no_proxy();
    }
    builder.build().map_err(|error| error.to_string())
}

impl SubscriptionSource for ReqwestSource {
    fn connect(
        &self,
        config: ValidatedSubscriptionConfig,
        since: Option<String>,
    ) -> BoxFuture<'static, Result<SubscriptionStream, String>> {
        if since
            .as_deref()
            .is_some_and(|cursor| !is_valid_message_id(cursor))
        {
            return Box::pin(async { Err("订阅游标格式无效".to_string()) });
        }
        let selected_client = match (config.endpoint.loopback_http, config.allow_insecure_http) {
            (true, true) => &self.loopback_insecure_client,
            (true, false) => &self.loopback_secure_client,
            (false, true) => &self.insecure_client,
            (false, false) => &self.secure_client,
        };
        let client = match selected_client {
            Ok(client) => client.clone(),
            Err(error) => {
                let error = error.clone();
                return Box::pin(async move { Err(error) });
            }
        };

        Box::pin(async move {
            let mut subscription_url = config.endpoint.subscription;
            if let Some(since) = since.filter(|cursor| !cursor.is_empty()) {
                subscription_url
                    .query_pairs_mut()
                    .append_pair("since", &since);
            }
            let mut request = client
                .get(subscription_url)
                .header("Accept", "text/event-stream");
            if !config.username.is_empty() {
                request = request.basic_auth(&config.username, Some(&config.password));
            }
            let response = request.send().await.map_err(|error| error.to_string())?;
            if !response.status().is_success() {
                return Err(format!("HTTP {}", response.status()));
            }
            let stream = response.bytes_stream().map(|result| {
                result
                    .map(|bytes| bytes.to_vec())
                    .map_err(|error| error.to_string())
            });
            Ok(Box::pin(stream) as SubscriptionStream)
        })
    }
}

#[derive(Clone, Copy)]
struct RetryPolicy {
    initial: Duration,
    maximum: Duration,
    stream_timeout: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            initial: Duration::from_secs(5),
            maximum: Duration::from_secs(300),
            stream_timeout: Duration::from_secs(120),
        }
    }
}

#[derive(Clone)]
pub(crate) struct SubscriptionCore {
    source: Arc<dyn SubscriptionSource>,
    store: Arc<dyn SubscriptionStore>,
    commit_gate: Arc<Mutex<()>>,
    retry: RetryPolicy,
}

impl Default for SubscriptionCore {
    fn default() -> Self {
        Self {
            source: Arc::new(ReqwestSource::default()),
            store: Arc::new(crate::history::SqliteSubscriptionStore),
            commit_gate: Arc::new(Mutex::new(())),
            retry: RetryPolicy::default(),
        }
    }
}

impl SubscriptionCore {
    async fn run(self, config: SubscriptionConfig, context: RunContext) {
        if context.is_cancelled_or_stale() {
            return;
        }
        let config = match ValidatedSubscriptionConfig::try_from(config) {
            Ok(config) => config,
            Err(error) => {
                eprintln!("[ntfy] 订阅配置无效：{error}");
                context
                    .publish_state(SubscriptionState::ConfigurationError)
                    .await;
                return;
            }
        };

        let mut delay = self.retry.initial;
        if !context.publish_state(SubscriptionState::Connecting).await {
            return;
        }

        loop {
            if context.is_cancelled_or_stale() {
                return;
            }
            let connection = match self.store.load_cursor(&config.key) {
                Ok(since) => {
                    if context.is_cancelled_or_stale() {
                        return;
                    }
                    tokio::select! {
                        _ = context.cancellation.cancelled() => return,
                        connection = self.source.connect(config.clone(), since) => connection,
                    }
                }
                Err(error) => Err(format!("读取订阅游标失败：{error}")),
            };

            let opened = match connection {
                Ok(stream) => self.consume_stream(stream, &config.key, &context).await,
                Err(error) => {
                    eprintln!("[ntfy] SSE 连接失败：{error}");
                    false
                }
            };

            if context.is_cancelled_or_stale() {
                return;
            }
            if opened {
                delay = self.retry.initial;
            }
            if !context.publish_state(SubscriptionState::Retrying).await {
                return;
            }

            if !context.wait(delay).await {
                return;
            }
            delay = (delay * 2).min(self.retry.maximum);
            if !context.publish_state(SubscriptionState::Connecting).await {
                return;
            }
        }
    }

    async fn consume_stream(
        &self,
        mut stream: SubscriptionStream,
        key: &SubscriptionKey,
        context: &RunContext,
    ) -> bool {
        let mut decoder = SseDecoder::new();
        let mut opened = false;

        loop {
            let next = tokio::select! {
                _ = context.cancellation.cancelled() => return opened,
                next = tokio::time::timeout(self.retry.stream_timeout, stream.next()) => next,
            };
            let chunk = match next {
                Ok(Some(Ok(chunk))) => chunk,
                Ok(Some(Err(error))) => {
                    eprintln!("[ntfy] SSE 数据流错误：{error}");
                    break;
                }
                Ok(None) => break,
                Err(_) => {
                    eprintln!("[ntfy] SSE 数据流超时");
                    break;
                }
            };

            let mut input = chunk.as_slice();
            while !input.is_empty() {
                let Some(outcome) = decoder.next_event(&mut input) else {
                    break;
                };
                match self
                    .handle_decoded_event(outcome, key, context, &mut opened)
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => return opened,
                    Err(error) => {
                        eprintln!("[ntfy] 消息处理失败：{error}");
                        return opened;
                    }
                }
            }
        }

        if let Some(outcome) = decoder.finish() {
            match self
                .handle_decoded_event(outcome, key, context, &mut opened)
                .await
            {
                Ok(true) => {}
                Ok(false) => return opened,
                Err(error) => eprintln!("[ntfy] 消息处理失败：{error}"),
            }
        }

        opened
    }

    async fn handle_decoded_event(
        &self,
        outcome: DecodeOutcome,
        key: &SubscriptionKey,
        context: &RunContext,
        opened: &mut bool,
    ) -> Result<bool, String> {
        let event = match outcome {
            DecodeOutcome::Data(data) => parse_event(&data),
            DecodeOutcome::Dropped(_) => None,
        };
        match event {
            Some(IncomingEvent::Open) => {
                if !*opened {
                    *opened = true;
                    if !context.publish_state(SubscriptionState::Connected).await {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Some(IncomingEvent::Message(message)) => {
                context.deliver_message(&*self.store, &self.commit_gate, key, message)
            }
            None => Ok(true),
        }
    }
}

struct ActiveSubscription {
    cancellation: CancellationToken,
    sink: Arc<dyn SubscriptionSink>,
}

enum StateDelivery {
    Current {
        generation: u64,
        state: SubscriptionState,
        sink: Arc<dyn SubscriptionSink>,
        acknowledged: oneshot::Sender<()>,
    },
    Forced {
        state: SubscriptionState,
        sink: Arc<dyn SubscriptionSink>,
    },
}

struct ControllerState {
    active: Option<ActiveSubscription>,
    receiver: Option<mpsc::UnboundedReceiver<StateDelivery>>,
}

pub(crate) struct SubscriptionController {
    control: Mutex<ControllerState>,
    generation: Arc<AtomicU64>,
    state_sender: mpsc::UnboundedSender<StateDelivery>,
    #[cfg(test)]
    queued_current_states: Arc<AtomicUsize>,
}

impl Default for SubscriptionController {
    fn default() -> Self {
        let (state_sender, receiver) = mpsc::unbounded_channel();
        Self {
            control: Mutex::new(ControllerState {
                active: None,
                receiver: Some(receiver),
            }),
            generation: Arc::new(AtomicU64::new(0)),
            state_sender,
            #[cfg(test)]
            queued_current_states: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl SubscriptionController {
    pub(crate) fn reconfigure<S, F>(
        &self,
        core: SubscriptionCore,
        config: SubscriptionConfig,
        sink: S,
        mut spawn: F,
    ) where
        S: SubscriptionSink + 'static,
        F: FnMut(BoxFuture<'static, ()>),
    {
        let sink: Arc<dyn SubscriptionSink> = Arc::new(sink);
        let cancellation = CancellationToken::new();

        let (dispatcher, worker) = {
            let mut control = self.control.lock().unwrap();
            let generation = self
                .generation
                .fetch_add(1, Ordering::SeqCst)
                .wrapping_add(1);
            if let Some(old) = control.active.take() {
                old.cancellation.cancel();
                let _ = self.state_sender.send(StateDelivery::Forced {
                    state: SubscriptionState::Stopped,
                    sink: old.sink,
                });
            }
            control.active = Some(ActiveSubscription {
                cancellation: cancellation.clone(),
                sink: Arc::clone(&sink),
            });
            let context = RunContext {
                generation,
                current_generation: Arc::clone(&self.generation),
                cancellation,
                sink,
                state_sender: self.state_sender.clone(),
                #[cfg(test)]
                queued_current_states: Arc::clone(&self.queued_current_states),
            };
            let dispatcher = control.receiver.take().map(|receiver| {
                state_dispatcher(
                    receiver,
                    Arc::clone(&self.generation),
                    #[cfg(test)]
                    Arc::clone(&self.queued_current_states),
                )
            });
            let worker = Box::pin(async move {
                core.run(config, context).await;
            }) as BoxFuture<'static, ()>;
            (dispatcher, worker)
        };

        if let Some(dispatcher) = dispatcher {
            spawn(dispatcher);
        }
        spawn(worker);
    }

    pub(crate) fn stop(&self) {
        let mut control = self.control.lock().unwrap();
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(active) = control.active.take() {
            active.cancellation.cancel();
            let _ = self.state_sender.send(StateDelivery::Forced {
                state: SubscriptionState::Stopped,
                sink: active.sink,
            });
        }
    }
}

fn state_dispatcher(
    mut receiver: mpsc::UnboundedReceiver<StateDelivery>,
    current_generation: Arc<AtomicU64>,
    #[cfg(test)] queued_current_states: Arc<AtomicUsize>,
) -> BoxFuture<'static, ()> {
    Box::pin(async move {
        while let Some(delivery) = receiver.recv().await {
            let (state, sink, acknowledged) = match delivery {
                StateDelivery::Current {
                    generation,
                    state,
                    sink,
                    acknowledged,
                } => {
                    #[cfg(test)]
                    queued_current_states.fetch_sub(1, Ordering::SeqCst);
                    if current_generation.load(Ordering::SeqCst) != generation {
                        let _ = acknowledged.send(());
                        continue;
                    }
                    (state, sink, Some(acknowledged))
                }
                StateDelivery::Forced { state, sink } => (state, sink, None),
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                sink.state_changed(state);
            }));
            if let Some(acknowledged) = acknowledged {
                let _ = acknowledged.send(());
            }
            if result.is_err() {
                eprintln!("[ntfy] 状态回调发生 panic");
            }
        }
    })
}

struct RunContext {
    generation: u64,
    current_generation: Arc<AtomicU64>,
    cancellation: CancellationToken,
    sink: Arc<dyn SubscriptionSink>,
    state_sender: mpsc::UnboundedSender<StateDelivery>,
    #[cfg(test)]
    queued_current_states: Arc<AtomicUsize>,
}

impl RunContext {
    fn is_cancelled_or_stale(&self) -> bool {
        self.cancellation.is_cancelled()
            || self.current_generation.load(Ordering::SeqCst) != self.generation
    }

    async fn publish_state(&self, next: SubscriptionState) -> bool {
        if self.is_cancelled_or_stale() {
            return false;
        }
        let (acknowledged, acknowledgment) = oneshot::channel();
        #[cfg(test)]
        self.queued_current_states.fetch_add(1, Ordering::SeqCst);
        if self
            .state_sender
            .send(StateDelivery::Current {
                generation: self.generation,
                state: next,
                sink: Arc::clone(&self.sink),
                acknowledged,
            })
            .is_err()
        {
            #[cfg(test)]
            self.queued_current_states.fetch_sub(1, Ordering::SeqCst);
            return false;
        }
        tokio::select! {
            _ = self.cancellation.cancelled() => false,
            result = acknowledgment => result.is_ok() && !self.is_cancelled_or_stale(),
        }
    }

    fn generation_guard(&self) -> GenerationGuard {
        GenerationGuard {
            generation: self.generation,
            current_generation: Arc::clone(&self.current_generation),
            cancellation: self.cancellation.clone(),
        }
    }

    fn deliver_message(
        &self,
        store: &dyn SubscriptionStore,
        commit_gate: &Mutex<()>,
        key: &SubscriptionKey,
        message: SubscriptionMessage,
    ) -> Result<bool, String> {
        let commit = {
            // 仅序列化 generation 判定与持久化。平台 Sink 属于外部回调，
            // 可能阻塞或重入控制器，绝不能在持有提交锁时调用。
            let _commit = commit_gate
                .lock()
                .map_err(|_| "订阅提交锁不可用".to_string())?;
            if self.is_cancelled_or_stale() {
                return Ok(false);
            }
            if validate_persistable_message(&message).is_err() {
                return Ok(true);
            }
            if message.topic != key.topic {
                eprintln!("[ntfy] 忽略主题不匹配的事件");
                return Ok(true);
            }
            store.commit_message(key, &message, &self.generation_guard())?
        };

        match commit {
            StoreCommit::Inserted => {
                // Store 在自己的串行临界区内确认 generation 后，消息即被接受。
                // 即使此刻发生重配置也必须完成一次通知，否则新任务会从已推进的
                // cursor 继续，造成这条消息永久不可见。
                if let Err(error) = self.sink.message_received(message) {
                    eprintln!("[ntfy] 平台通知失败：{error}");
                }
                Ok(!self.is_cancelled_or_stale())
            }
            StoreCommit::Duplicate => Ok(!self.is_cancelled_or_stale()),
            StoreCommit::StaleGeneration => Ok(false),
        }
    }

    async fn wait(&self, duration: Duration) -> bool {
        tokio::select! {
            _ = self.cancellation.cancelled() => false,
            _ = tokio::time::sleep(duration) => !self.is_cancelled_or_stale(),
        }
    }
}

enum IncomingEvent {
    Open,
    Message(SubscriptionMessage),
}

#[derive(Deserialize)]
struct WireEvent {
    event: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    topic: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

pub(crate) fn is_valid_message_id(id: &str) -> bool {
    id.len() == 12 && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn parse_event(data: &[u8]) -> Option<IncomingEvent> {
    if data.len() > MAX_NTFY_JSON_BYTES {
        return None;
    }
    let event: WireEvent = serde_json::from_slice(data).ok()?;
    match event.event.as_str() {
        "open" => Some(IncomingEvent::Open),
        "message" => {
            let id = event.id?;
            if !is_valid_message_id(&id) {
                return None;
            }
            let message = SubscriptionMessage {
                id,
                topic: event.topic?,
                title: event.title.unwrap_or_else(|| "ntfy 消息".to_string()),
                message: event.message.unwrap_or_else(|| "triggered".to_string()),
            };
            validate_persistable_message(&message).ok()?;
            Some(IncomingEvent::Message(message))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Condvar, Mutex};
    use std::time::Instant;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Notify;

    const CURSOR_0: &str = "000000000000";
    const CURSOR_1: &str = "000000000001";

    enum ConnectPlan {
        Fail(&'static str),
        Pending,
        Stream(mpsc::UnboundedReceiver<Result<Vec<u8>, String>>),
    }

    #[derive(Clone)]
    struct ConnectAttempt {
        config: ValidatedSubscriptionConfig,
        since: Option<String>,
    }

    #[derive(Default)]
    struct MockSource {
        plans: Mutex<VecDeque<ConnectPlan>>,
        attempts: Mutex<Vec<ConnectAttempt>>,
        changed: Notify,
    }

    impl MockSource {
        fn with_plans(plans: impl IntoIterator<Item = ConnectPlan>) -> Arc<Self> {
            Arc::new(Self {
                plans: Mutex::new(plans.into_iter().collect()),
                ..Self::default()
            })
        }

        fn attempts(&self) -> Vec<ConnectAttempt> {
            self.attempts.lock().unwrap().clone()
        }

        async fn wait_for_attempts(&self, expected: usize) {
            wait_until(&self.changed, || self.attempts().len() >= expected).await;
        }
    }

    impl SubscriptionSource for MockSource {
        fn connect(
            &self,
            config: ValidatedSubscriptionConfig,
            since: Option<String>,
        ) -> BoxFuture<'static, Result<SubscriptionStream, String>> {
            self.attempts
                .lock()
                .unwrap()
                .push(ConnectAttempt { config, since });
            self.changed.notify_waiters();
            let plan = self.plans.lock().unwrap().pop_front();
            Box::pin(async move {
                match plan {
                    Some(ConnectPlan::Fail(error)) => Err(error.to_string()),
                    Some(ConnectPlan::Pending) => {
                        std::future::pending::<Result<SubscriptionStream, String>>().await
                    }
                    Some(ConnectPlan::Stream(receiver)) => {
                        let stream = stream::unfold(receiver, |mut receiver| async move {
                            receiver.recv().await.map(|item| (item, receiver))
                        });
                        Ok(Box::pin(stream) as SubscriptionStream)
                    }
                    None => Err("no mock connection plan".to_string()),
                }
            })
        }
    }

    #[derive(Default)]
    struct MockStoreState {
        cursors: HashMap<SubscriptionKey, String>,
        message_ids: HashSet<String>,
    }

    #[derive(Default)]
    struct MockStore {
        state: Mutex<MockStoreState>,
        fail_load: AtomicBool,
        fail_commit: AtomicBool,
        changed: Notify,
    }

    impl MockStore {
        fn set_cursor(&self, key: SubscriptionKey, cursor: &str) {
            self.state
                .lock()
                .unwrap()
                .cursors
                .insert(key, cursor.to_string());
            self.changed.notify_waiters();
        }

        fn cursor(&self, key: &SubscriptionKey) -> Option<String> {
            self.state.lock().unwrap().cursors.get(key).cloned()
        }

        async fn wait_for_cursor(&self, key: &SubscriptionKey, expected: &str) {
            wait_until(&self.changed, || {
                self.cursor(key).as_deref() == Some(expected)
            })
            .await;
        }
    }

    impl SubscriptionStore for MockStore {
        fn load_cursor(&self, key: &SubscriptionKey) -> Result<Option<String>, String> {
            if self.fail_load.load(Ordering::SeqCst) {
                return Err("cursor load failed".to_string());
            }
            Ok(self.state.lock().unwrap().cursors.get(key).cloned())
        }

        fn commit_message(
            &self,
            key: &SubscriptionKey,
            message: &SubscriptionMessage,
            generation: &GenerationGuard,
        ) -> Result<StoreCommit, String> {
            let mut state = self.state.lock().unwrap();
            if !generation.is_current() {
                return Ok(StoreCommit::StaleGeneration);
            }
            if self.fail_commit.load(Ordering::SeqCst) {
                return Err("message commit failed".to_string());
            }
            if message.id.is_empty() {
                return Ok(StoreCommit::Duplicate);
            }
            let inserted = state.message_ids.insert(message.id.clone());
            state.cursors.insert(key.clone(), message.id.clone());
            drop(state);
            self.changed.notify_waiters();
            Ok(if inserted {
                StoreCommit::Inserted
            } else {
                StoreCommit::Duplicate
            })
        }
    }

    #[derive(Default)]
    struct BlockingStore {
        cursor: Mutex<Option<String>>,
        checked: AtomicBool,
        checked_changed: Notify,
        released: Mutex<bool>,
        release_changed: Condvar,
    }

    impl BlockingStore {
        async fn wait_until_checked(&self) {
            wait_until(&self.checked_changed, || {
                self.checked.load(Ordering::SeqCst)
            })
            .await;
        }

        fn release(&self) {
            *self.released.lock().unwrap() = true;
            self.release_changed.notify_all();
        }
    }

    impl SubscriptionStore for BlockingStore {
        fn load_cursor(&self, _key: &SubscriptionKey) -> Result<Option<String>, String> {
            Ok(self.cursor.lock().unwrap().clone())
        }

        fn commit_message(
            &self,
            _key: &SubscriptionKey,
            message: &SubscriptionMessage,
            generation: &GenerationGuard,
        ) -> Result<StoreCommit, String> {
            let mut cursor = self.cursor.lock().unwrap();
            if !generation.is_current() {
                return Ok(StoreCommit::StaleGeneration);
            }
            self.checked.store(true, Ordering::SeqCst);
            self.checked_changed.notify_waiters();
            let released = self.released.lock().unwrap();
            let _released = self
                .release_changed
                .wait_while(released, |released| !*released)
                .unwrap();
            *cursor = Some(message.id.clone());
            Ok(StoreCommit::Inserted)
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        states: Mutex<Vec<SubscriptionState>>,
        messages: Mutex<Vec<SubscriptionMessage>>,
        events: Mutex<Vec<String>>,
        fail_messages: bool,
        changed: Notify,
    }

    impl RecordingSink {
        fn states(&self) -> Vec<SubscriptionState> {
            self.states.lock().unwrap().clone()
        }

        fn message_count(&self) -> usize {
            self.messages.lock().unwrap().len()
        }

        fn last_state(&self) -> Option<SubscriptionState> {
            self.states.lock().unwrap().last().copied()
        }

        fn events(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }

        async fn wait_for_state(&self, expected: SubscriptionState) {
            wait_until(&self.changed, || self.states().contains(&expected)).await;
        }

        async fn wait_for_messages(&self, expected: usize) {
            wait_until(&self.changed, || self.message_count() >= expected).await;
        }
    }

    impl SubscriptionSink for Arc<RecordingSink> {
        fn state_changed(&self, state: SubscriptionState) {
            self.states.lock().unwrap().push(state);
            self.events.lock().unwrap().push(format!("state:{state:?}"));
            self.changed.notify_waiters();
        }

        fn message_received(&self, message: SubscriptionMessage) -> Result<(), String> {
            if self.fail_messages {
                return Err("sink rejected message".to_string());
            }
            self.events
                .lock()
                .unwrap()
                .push(format!("message:{}", message.id));
            self.messages.lock().unwrap().push(message);
            self.changed.notify_waiters();
            Ok(())
        }
    }

    #[derive(Default)]
    struct BlockingMessageSink {
        entered: AtomicBool,
        entered_changed: Notify,
        released: Mutex<bool>,
        release_changed: Condvar,
        messages: AtomicUsize,
    }

    impl BlockingMessageSink {
        async fn wait_until_entered(&self) {
            wait_until(&self.entered_changed, || {
                self.entered.load(Ordering::SeqCst)
            })
            .await;
        }

        fn release(&self) {
            *self.released.lock().unwrap() = true;
            self.release_changed.notify_all();
        }
    }

    impl SubscriptionSink for Arc<BlockingMessageSink> {
        fn state_changed(&self, _state: SubscriptionState) {}

        fn message_received(&self, _message: SubscriptionMessage) -> Result<(), String> {
            self.entered.store(true, Ordering::SeqCst);
            self.entered_changed.notify_waiters();
            let released = self.released.lock().unwrap();
            let _released = self
                .release_changed
                .wait_while(released, |released| !*released)
                .unwrap();
            self.messages.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct ReentrantStoppingSink {
        controller: std::sync::Weak<SubscriptionController>,
        recording: Arc<RecordingSink>,
    }

    impl SubscriptionSink for ReentrantStoppingSink {
        fn state_changed(&self, state: SubscriptionState) {
            self.recording.states.lock().unwrap().push(state);
            self.recording
                .events
                .lock()
                .unwrap()
                .push(format!("state:{state:?}"));
            self.recording.changed.notify_waiters();
            if state == SubscriptionState::Connecting {
                if let Some(controller) = self.controller.upgrade() {
                    controller.stop();
                }
            }
        }

        fn message_received(&self, message: SubscriptionMessage) -> Result<(), String> {
            self.recording
                .events
                .lock()
                .unwrap()
                .push(format!("message:{}", message.id));
            self.recording.messages.lock().unwrap().push(message);
            self.recording.changed.notify_waiters();
            Ok(())
        }
    }

    async fn wait_until(changed: &Notify, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let notified = changed.notified();
            if condition() {
                return;
            }
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .expect("timed out waiting for subscription state");
            tokio::time::timeout(remaining, notified)
                .await
                .expect("timed out waiting for subscription state");
        }
    }

    fn config(server: &str) -> SubscriptionConfig {
        SubscriptionConfig {
            server: server.to_string(),
            username: String::new(),
            password: String::new(),
            topic: "test-topic".to_string(),
            allow_insecure_http: false,
        }
    }

    fn key(server: &str) -> SubscriptionKey {
        ValidatedSubscriptionConfig::try_from(config(server))
            .unwrap()
            .key
    }

    fn channel_plan() -> (mpsc::UnboundedSender<Result<Vec<u8>, String>>, ConnectPlan) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (sender, ConnectPlan::Stream(receiver))
    }

    fn core_with_store<S>(source: Arc<MockSource>, store: Arc<S>) -> SubscriptionCore
    where
        S: SubscriptionStore + 'static,
    {
        SubscriptionCore {
            source,
            store,
            commit_gate: Arc::new(Mutex::new(())),
            retry: RetryPolicy {
                initial: Duration::from_millis(100),
                maximum: Duration::from_millis(200),
                stream_timeout: Duration::from_secs(2),
            },
        }
    }

    fn core(source: Arc<MockSource>) -> SubscriptionCore {
        core_with_store(source, Arc::new(MockStore::default()))
    }

    fn send(sender: &mpsc::UnboundedSender<Result<Vec<u8>, String>>, event: &str) {
        sender.send(Ok(event.as_bytes().to_vec())).unwrap();
    }

    fn spawn_subscription<S>(
        controller: &SubscriptionController,
        core: SubscriptionCore,
        config: SubscriptionConfig,
        sink: S,
    ) -> tokio::task::JoinHandle<()>
    where
        S: SubscriptionSink + 'static,
    {
        let mut handles = Vec::new();
        controller.reconfigure(core, config, sink, |task| {
            handles.push(tokio::spawn(task));
        });
        handles.pop().expect("subscription worker was not spawned")
    }

    #[tokio::test]
    async fn http_success_does_not_connect_until_open_event() {
        let (sender, plan) = channel_plan();
        let source = MockSource::with_plans([plan]);
        let controller = SubscriptionController::default();
        let sink = Arc::new(RecordingSink::default());
        let task = spawn_subscription(
            &controller,
            core(Arc::clone(&source)),
            config("https://example.test"),
            Arc::clone(&sink),
        );

        sink.wait_for_state(SubscriptionState::Connecting).await;
        send(
            &sender,
            "data: {\"id\":\"beforeopen01\",\"event\":\"message\",\"topic\":\"test-topic\",\"message\":\"hello\"}\n\n",
        );
        sink.wait_for_messages(1).await;
        assert!(!sink.states().contains(&SubscriptionState::Connected));

        send(&sender, "data: {\"event\":\"open\"}\n\n");
        sink.wait_for_state(SubscriptionState::Connected).await;

        controller.stop();
        sink.wait_for_state(SubscriptionState::Stopped).await;
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sink.last_state(), Some(SubscriptionState::Stopped));
    }

    #[tokio::test]
    async fn connected_state_is_delivered_before_following_message() {
        let (sender, plan) = channel_plan();
        let source = MockSource::with_plans([plan]);
        let controller = SubscriptionController::default();
        let sink = Arc::new(RecordingSink::default());
        let task = spawn_subscription(
            &controller,
            core(source),
            config("https://example.test"),
            Arc::clone(&sink),
        );

        send(
            &sender,
            concat!(
                "data: {\"event\":\"open\"}\n\n",
                "data: {\"id\":\"afteropen001\",\"event\":\"message\",",
                "\"topic\":\"test-topic\",\"message\":\"hello\"}\n\n"
            ),
        );
        sink.wait_for_messages(1).await;

        let events = sink.events();
        let connected = events
            .iter()
            .position(|event| event == "state:Connected")
            .unwrap();
        let message = events
            .iter()
            .position(|event| event == "message:afteropen001")
            .unwrap();
        assert!(
            connected < message,
            "events were delivered out of order: {events:?}"
        );

        controller.stop();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn stream_end_after_open_reports_retrying() {
        let (sender, plan) = channel_plan();
        let source = MockSource::with_plans([plan]);
        let controller = SubscriptionController::default();
        let sink = Arc::new(RecordingSink::default());
        let task = spawn_subscription(
            &controller,
            core(source),
            config("https://example.test"),
            Arc::clone(&sink),
        );

        send(&sender, "data: {\"event\":\"open\"}\n\n");
        sink.wait_for_state(SubscriptionState::Connected).await;
        drop(sender);
        sink.wait_for_state(SubscriptionState::Retrying).await;
        assert_eq!(sink.last_state(), Some(SubscriptionState::Retrying));

        controller.stop();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn restart_cancels_backoff_and_uses_new_generation() {
        let (sender, second_plan) = channel_plan();
        let source = MockSource::with_plans([ConnectPlan::Fail("offline"), second_plan]);
        let controller = SubscriptionController::default();
        let first_sink = Arc::new(RecordingSink::default());
        let second_sink = Arc::new(RecordingSink::default());

        let first_task = spawn_subscription(
            &controller,
            core(Arc::clone(&source)),
            config("https://old.example"),
            Arc::clone(&first_sink),
        );
        first_sink.wait_for_state(SubscriptionState::Retrying).await;

        let second_task = spawn_subscription(
            &controller,
            core(Arc::clone(&source)),
            config("https://new.example"),
            Arc::clone(&second_sink),
        );
        first_sink.wait_for_state(SubscriptionState::Stopped).await;
        source.wait_for_attempts(2).await;
        assert_eq!(
            source.attempts()[1].config.endpoint.server.as_str(),
            "https://new.example/"
        );

        send(&sender, "data: {\"event\":\"open\"}\n\n");
        second_sink
            .wait_for_state(SubscriptionState::Connected)
            .await;

        controller.stop();
        second_sink.wait_for_state(SubscriptionState::Stopped).await;
        for task in [first_task, second_task] {
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn reconfigure_cancels_pending_connect_and_stops_old_sink() {
        let (sender, current_plan) = channel_plan();
        let source = MockSource::with_plans([ConnectPlan::Pending, current_plan]);
        let controller = SubscriptionController::default();
        let stale_sink = Arc::new(RecordingSink::default());
        let current_sink = Arc::new(RecordingSink::default());

        let stale_task = spawn_subscription(
            &controller,
            core(Arc::clone(&source)),
            config("https://pending.example"),
            Arc::clone(&stale_sink),
        );
        source.wait_for_attempts(1).await;
        stale_sink
            .wait_for_state(SubscriptionState::Connecting)
            .await;

        let current_task = spawn_subscription(
            &controller,
            core(Arc::clone(&source)),
            config("https://current.example"),
            Arc::clone(&current_sink),
        );
        stale_sink.wait_for_state(SubscriptionState::Stopped).await;
        source.wait_for_attempts(2).await;
        tokio::time::timeout(Duration::from_secs(1), stale_task)
            .await
            .unwrap()
            .unwrap();

        send(&sender, "data: {\"event\":\"open\"}\n\n");
        current_sink
            .wait_for_state(SubscriptionState::Connected)
            .await;
        assert_eq!(
            source.attempts()[1].config.endpoint.server.as_str(),
            "https://current.example/"
        );

        controller.stop();
        current_sink
            .wait_for_state(SubscriptionState::Stopped)
            .await;
        current_task.await.unwrap();
    }

    #[tokio::test]
    async fn state_sink_can_reenter_controller_without_deadlocking() {
        let source = MockSource::with_plans([ConnectPlan::Pending]);
        let controller = Arc::new(SubscriptionController::default());
        let recording = Arc::new(RecordingSink::default());
        let task = spawn_subscription(
            &controller,
            core(source),
            config("https://pending.example"),
            ReentrantStoppingSink {
                controller: Arc::downgrade(&controller),
                recording: Arc::clone(&recording),
            },
        );

        recording.wait_for_state(SubscriptionState::Stopped).await;
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recording.last_state(), Some(SubscriptionState::Stopped));
    }

    #[tokio::test]
    async fn stale_future_cannot_start_or_publish_after_reconfigure() {
        let (sender, plan) = channel_plan();
        let source = MockSource::with_plans([plan]);
        let controller = SubscriptionController::default();
        let stale_sink = Arc::new(RecordingSink::default());
        let current_sink = Arc::new(RecordingSink::default());

        let mut stale_tasks = Vec::new();
        controller.reconfigure(
            core(Arc::clone(&source)),
            config("https://stale.example"),
            Arc::clone(&stale_sink),
            |task| stale_tasks.push(task),
        );
        let stale_worker = stale_tasks
            .pop()
            .expect("stale subscription worker was not created");
        let dispatcher = stale_tasks.pop().expect("state dispatcher was not created");
        let stale_task = tokio::spawn(stale_worker);
        let deadline = Instant::now() + Duration::from_secs(1);
        while controller.queued_current_states.load(Ordering::SeqCst) < 1 {
            assert!(
                Instant::now() < deadline,
                "stale state was not queued before reconfigure"
            );
            tokio::task::yield_now().await;
        }
        let current_task = spawn_subscription(
            &controller,
            core(Arc::clone(&source)),
            config("https://current.example"),
            Arc::clone(&current_sink),
        );
        tokio::spawn(dispatcher);
        stale_sink.wait_for_state(SubscriptionState::Stopped).await;

        stale_task.await.unwrap();
        assert_eq!(stale_sink.states(), vec![SubscriptionState::Stopped]);

        source.wait_for_attempts(1).await;
        assert_eq!(
            source.attempts()[0].config.endpoint.server.as_str(),
            "https://current.example/"
        );
        send(&sender, "data: {\"event\":\"open\"}\n\n");
        current_sink
            .wait_for_state(SubscriptionState::Connected)
            .await;

        controller.stop();
        tokio::time::timeout(Duration::from_secs(1), current_task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn empty_configuration_reports_configuration_error_without_connecting() {
        let source = Arc::new(MockSource::default());
        let controller = SubscriptionController::default();
        let sink = Arc::new(RecordingSink::default());
        let task = spawn_subscription(
            &controller,
            core(Arc::clone(&source)),
            SubscriptionConfig::from(&Config::default()),
            Arc::clone(&sink),
        );

        sink.wait_for_state(SubscriptionState::ConfigurationError)
            .await;
        assert!(source.attempts().is_empty());
        controller.stop();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn invalid_configuration_never_reaches_network_source() {
        let invalid_configs = [
            SubscriptionConfig {
                server: "http://example.test".to_string(),
                username: String::new(),
                password: String::new(),
                topic: "alerts".to_string(),
                allow_insecure_http: false,
            },
            SubscriptionConfig {
                server: "https://user@example.test".to_string(),
                username: String::new(),
                password: String::new(),
                topic: "alerts".to_string(),
                allow_insecure_http: false,
            },
            SubscriptionConfig {
                server: "https://example.test".to_string(),
                username: String::new(),
                password: "secret".to_string(),
                topic: "alerts".to_string(),
                allow_insecure_http: false,
            },
            SubscriptionConfig {
                server: "https://example.test".to_string(),
                username: String::new(),
                password: String::new(),
                topic: "invalid/topic".to_string(),
                allow_insecure_http: false,
            },
        ];

        for invalid in invalid_configs {
            let source = Arc::new(MockSource::default());
            let controller = SubscriptionController::default();
            let sink = Arc::new(RecordingSink::default());
            let task = spawn_subscription(
                &controller,
                core(Arc::clone(&source)),
                invalid,
                Arc::clone(&sink),
            );

            sink.wait_for_state(SubscriptionState::ConfigurationError)
                .await;
            assert!(source.attempts().is_empty());
            controller.stop();
            task.await.unwrap();
        }
    }

    #[test]
    fn subscription_key_normalizes_equivalent_base_urls() {
        assert_eq!(
            key("https://EXAMPLE.test:443/base/"),
            key("https://example.test/base")
        );
        assert_ne!(
            key("https://example.test/base"),
            key("https://example.test/other")
        );
    }

    #[tokio::test]
    async fn first_connection_without_cursor_omits_since() {
        let source = MockSource::with_plans([ConnectPlan::Pending]);
        let store = Arc::new(MockStore::default());
        let controller = SubscriptionController::default();
        let sink = Arc::new(RecordingSink::default());
        let task = spawn_subscription(
            &controller,
            core_with_store(Arc::clone(&source), store),
            config("https://example.test/base"),
            Arc::clone(&sink),
        );

        source.wait_for_attempts(1).await;
        assert_eq!(source.attempts()[0].since, None);
        controller.stop();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn reconnect_uses_cursor_committed_by_previous_stream() {
        let (sender, first_plan) = channel_plan();
        let source = MockSource::with_plans([first_plan, ConnectPlan::Pending]);
        let store = Arc::new(MockStore::default());
        let subscription_key = key("https://example.test");
        store.set_cursor(subscription_key.clone(), CURSOR_0);
        let controller = SubscriptionController::default();
        let sink = Arc::new(RecordingSink::default());
        let task = spawn_subscription(
            &controller,
            core_with_store(Arc::clone(&source), Arc::clone(&store)),
            config("https://example.test"),
            Arc::clone(&sink),
        );

        source.wait_for_attempts(1).await;
        assert_eq!(source.attempts()[0].since.as_deref(), Some(CURSOR_0));
        send(&sender, "data: {\"event\":\"open\"}\n\n");
        send(
            &sender,
            "data: {\"id\":\"000000000001\",\"event\":\"message\",\"topic\":\"test-topic\",\"message\":\"hello\"}\n\n",
        );
        sink.wait_for_messages(1).await;
        store.wait_for_cursor(&subscription_key, CURSOR_1).await;
        drop(sender);
        source.wait_for_attempts(2).await;

        assert_eq!(source.attempts()[1].since.as_deref(), Some(CURSOR_1));
        assert_eq!(store.cursor(&subscription_key).as_deref(), Some(CURSOR_1));
        assert_eq!(source.attempts()[1].config.key, subscription_key);
        controller.stop();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn mismatched_event_topic_is_not_committed_or_notified() {
        let (sender, plan) = channel_plan();
        let source = MockSource::with_plans([plan]);
        let store = Arc::new(MockStore::default());
        let subscription_key = key("https://example.test");
        let controller = SubscriptionController::default();
        let sink = Arc::new(RecordingSink::default());
        let task = spawn_subscription(
            &controller,
            core_with_store(source, Arc::clone(&store)),
            config("https://example.test"),
            Arc::clone(&sink),
        );

        send(&sender, "data: {\"event\":\"open\"}\n\n");
        sink.wait_for_state(SubscriptionState::Connected).await;
        send(
            &sender,
            "data: {\"id\":\"000000000001\",\"event\":\"message\",\"topic\":\"spoofed\",\"message\":\"ignored\"}\n\n",
        );
        send(
            &sender,
            "data: {\"id\":\"000000000002\",\"event\":\"message\",\"topic\":\"test-topic\",\"message\":\"accepted\"}\n\n",
        );
        store
            .wait_for_cursor(&subscription_key, "000000000002")
            .await;
        sink.wait_for_messages(1).await;

        {
            let messages = sink.messages.lock().unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].id, "000000000002");
        }
        controller.stop();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_and_oversized_events_do_not_block_the_next_valid_message() {
        let (sender, plan) = channel_plan();
        let source = MockSource::with_plans([plan]);
        let store = Arc::new(MockStore::default());
        let subscription_key = key("https://example.test");
        store.set_cursor(subscription_key.clone(), CURSOR_0);
        let controller = SubscriptionController::default();
        let sink = Arc::new(RecordingSink::default());
        let task = spawn_subscription(
            &controller,
            core_with_store(source, Arc::clone(&store)),
            config("https://example.test"),
            Arc::clone(&sink),
        );

        send(&sender, "data: {\"event\":\"open\"}\n\n");
        sink.wait_for_state(SubscriptionState::Connected).await;
        let oversized = serde_json::to_string(&serde_json::json!({
            "event": "message",
            "id": "000000000001",
            "topic": "test-topic",
            "title": "x".repeat(MAX_TITLE_BYTES + 1),
            "message": "must be dropped"
        }))
        .unwrap();
        send(
            &sender,
            &format!(
                "data: {oversized}\n\ndata: {{broken json}}\n\ndata: {{\"event\":\"message\",\"id\":\"000000000002\",\"topic\":\"test-topic\",\"message\":\"accepted\"}}\n\n"
            ),
        );

        store
            .wait_for_cursor(&subscription_key, "000000000002")
            .await;
        sink.wait_for_messages(1).await;
        {
            let messages = sink.messages.lock().unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].id, "000000000002");
        }

        controller.stop();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn oversized_last_message_does_not_advance_cursor_before_reconnect() {
        let (sender, first_plan) = channel_plan();
        let source = MockSource::with_plans([first_plan, ConnectPlan::Pending]);
        let store = Arc::new(MockStore::default());
        let subscription_key = key("https://example.test");
        store.set_cursor(subscription_key.clone(), CURSOR_0);
        let controller = SubscriptionController::default();
        let sink = Arc::new(RecordingSink::default());
        let task = spawn_subscription(
            &controller,
            core_with_store(Arc::clone(&source), Arc::clone(&store)),
            config("https://example.test"),
            Arc::clone(&sink),
        );

        send(&sender, "data: {\"event\":\"open\"}\n\n");
        sink.wait_for_state(SubscriptionState::Connected).await;
        let oversized = serde_json::to_string(&serde_json::json!({
            "event": "message",
            "id": "000000000001",
            "topic": "test-topic",
            "message": "x".repeat(MAX_MESSAGE_BYTES + 1)
        }))
        .unwrap();
        send(&sender, &format!("data: {oversized}\n\n"));
        drop(sender);
        source.wait_for_attempts(2).await;

        assert_eq!(sink.message_count(), 0);
        assert_eq!(store.cursor(&subscription_key).as_deref(), Some(CURSOR_0));
        assert_eq!(source.attempts()[1].since.as_deref(), Some(CURSOR_0));

        controller.stop();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn failed_store_commit_does_not_notify_or_advance_cursor() {
        let (sender, first_plan) = channel_plan();
        let source = MockSource::with_plans([first_plan, ConnectPlan::Pending]);
        let store = Arc::new(MockStore::default());
        let subscription_key = key("https://example.test");
        store.set_cursor(subscription_key.clone(), CURSOR_0);
        store.fail_commit.store(true, Ordering::SeqCst);
        let controller = SubscriptionController::default();
        let sink = Arc::new(RecordingSink::default());
        let task = spawn_subscription(
            &controller,
            core_with_store(Arc::clone(&source), Arc::clone(&store)),
            config("https://example.test"),
            Arc::clone(&sink),
        );

        send(&sender, "data: {\"event\":\"open\"}\n\n");
        sink.wait_for_state(SubscriptionState::Connected).await;
        send(
            &sender,
            "data: {\"id\":\"000000000001\",\"event\":\"message\",\"topic\":\"test-topic\",\"message\":\"hello\"}\n\n",
        );
        sink.wait_for_state(SubscriptionState::Retrying).await;
        source.wait_for_attempts(2).await;

        assert_eq!(sink.message_count(), 0);
        assert_eq!(store.cursor(&subscription_key).as_deref(), Some(CURSOR_0));
        assert_eq!(source.attempts()[1].since.as_deref(), Some(CURSOR_0));
        controller.stop();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn cursor_load_failure_retries_without_contacting_source() {
        let source = Arc::new(MockSource::default());
        let store = Arc::new(MockStore::default());
        store.fail_load.store(true, Ordering::SeqCst);
        let controller = SubscriptionController::default();
        let sink = Arc::new(RecordingSink::default());
        let task = spawn_subscription(
            &controller,
            core_with_store(Arc::clone(&source), store),
            config("https://example.test"),
            Arc::clone(&sink),
        );

        sink.wait_for_state(SubscriptionState::Retrying).await;
        assert!(source.attempts().is_empty());
        controller.stop();
        task.await.unwrap();
    }

    #[test]
    fn stale_generation_cannot_commit_cursor() {
        let store = MockStore::default();
        let subscription_key = key("https://example.test");
        let generation = GenerationGuard {
            generation: 1,
            current_generation: Arc::new(AtomicU64::new(2)),
            cancellation: CancellationToken::new(),
        };
        let message = SubscriptionMessage {
            id: CURSOR_1.to_string(),
            topic: "test-topic".to_string(),
            title: "title".to_string(),
            message: "body".to_string(),
        };

        assert_eq!(
            store
                .commit_message(&subscription_key, &message, &generation)
                .unwrap(),
            StoreCommit::StaleGeneration
        );
        assert_eq!(store.cursor(&subscription_key), None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn accepted_commit_is_not_lost_when_reconfigured_mid_transaction() {
        let (sender, first_plan) = channel_plan();
        let source = MockSource::with_plans([first_plan, ConnectPlan::Pending]);
        let store = Arc::new(BlockingStore::default());
        let core = SubscriptionCore {
            source: source.clone(),
            store: store.clone(),
            commit_gate: Arc::new(Mutex::new(())),
            retry: RetryPolicy {
                initial: Duration::from_millis(100),
                maximum: Duration::from_millis(200),
                stream_timeout: Duration::from_secs(2),
            },
        };
        let controller = SubscriptionController::default();
        let old_sink = Arc::new(RecordingSink::default());
        let new_sink = Arc::new(RecordingSink::default());
        let old_task = spawn_subscription(
            &controller,
            core.clone(),
            config("https://example.test"),
            Arc::clone(&old_sink),
        );
        source.wait_for_attempts(1).await;
        send(&sender, "data: {\"event\":\"open\"}\n\n");
        old_sink.wait_for_state(SubscriptionState::Connected).await;
        send(
            &sender,
            "data: {\"id\":\"000000000001\",\"event\":\"message\",\"topic\":\"test-topic\",\"message\":\"accepted\"}\n\n",
        );
        store.wait_until_checked().await;

        let new_task = spawn_subscription(
            &controller,
            core,
            config("https://example.test"),
            Arc::clone(&new_sink),
        );
        store.release();

        old_sink.wait_for_messages(1).await;
        source.wait_for_attempts(2).await;
        assert_eq!(old_sink.message_count(), 1);
        assert_eq!(new_sink.message_count(), 0);
        assert_eq!(source.attempts()[1].since.as_deref(), Some(CURSOR_1));

        controller.stop();
        old_task.await.unwrap();
        new_task.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn blocking_old_sink_does_not_block_new_generation_commit() {
        let (old_sender, old_plan) = channel_plan();
        let (new_sender, new_plan) = channel_plan();
        let source = MockSource::with_plans([old_plan, new_plan]);
        let store = Arc::new(MockStore::default());
        let core = core_with_store(Arc::clone(&source), Arc::clone(&store));
        let subscription_key = key("https://example.test");
        let controller = SubscriptionController::default();
        let old_sink = Arc::new(BlockingMessageSink::default());
        let new_sink = Arc::new(RecordingSink::default());
        let old_task = spawn_subscription(
            &controller,
            core.clone(),
            config("https://example.test"),
            Arc::clone(&old_sink),
        );
        source.wait_for_attempts(1).await;
        send(&old_sender, "data: {\"event\":\"open\"}\n\n");
        send(
            &old_sender,
            "data: {\"id\":\"000000000001\",\"event\":\"message\",\"topic\":\"test-topic\",\"message\":\"old\"}\n\n",
        );
        old_sink.wait_until_entered().await;

        let new_task = spawn_subscription(
            &controller,
            core,
            config("https://example.test"),
            Arc::clone(&new_sink),
        );
        source.wait_for_attempts(2).await;
        assert_eq!(source.attempts()[1].since.as_deref(), Some(CURSOR_1));
        send(&new_sender, "data: {\"event\":\"open\"}\n\n");
        send(
            &new_sender,
            "data: {\"id\":\"000000000002\",\"event\":\"message\",\"topic\":\"test-topic\",\"message\":\"new\"}\n\n",
        );

        let delivered = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let notified = new_sink.changed.notified();
                if new_sink.message_count() == 1 {
                    break;
                }
                notified.await;
            }
        })
        .await;
        old_sink.release();

        assert!(
            delivered.is_ok(),
            "new generation was blocked by the old platform callback"
        );
        assert_eq!(
            store.cursor(&subscription_key).as_deref(),
            Some("000000000002")
        );
        controller.stop();
        old_task.await.unwrap();
        new_task.await.unwrap();
        assert_eq!(old_sink.messages.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn reserved_or_malformed_message_ids_are_not_parsed_as_messages() {
        for id in ["", "all", "latest", "10m", "has/slash", "12345678901"] {
            let data = format!(
                "{{\"id\":\"{id}\",\"event\":\"message\",\"topic\":\"test-topic\",\"message\":\"ignored\"}}"
            );
            assert!(parse_event(data.as_bytes()).is_none(), "{id}");
        }
        assert!(parse_event(
            b"{\"id\":\"AbCdEf123456\",\"event\":\"message\",\"topic\":\"test-topic\",\"message\":\"ok\"}"
        )
        .is_some());
    }

    #[test]
    fn json_and_persisted_fields_enforce_inclusive_byte_limits() {
        let mut exact_json = br#"{"event":"open"}"#.to_vec();
        exact_json.resize(MAX_NTFY_JSON_BYTES, b' ');
        assert!(matches!(
            parse_event(&exact_json),
            Some(IncomingEvent::Open)
        ));
        exact_json.push(b' ');
        assert!(parse_event(&exact_json).is_none());

        let make_message = |title: String, message: String| {
            serde_json::to_vec(&serde_json::json!({
                "event": "message",
                "id": "000000000001",
                "topic": "test-topic",
                "title": title,
                "message": message,
                "attachment": { "name": "ignored future-compatible field" }
            }))
            .unwrap()
        };

        assert!(parse_event(&make_message(
            "t".repeat(MAX_TITLE_BYTES),
            "m".repeat(MAX_MESSAGE_BYTES)
        ))
        .is_some());
        assert!(parse_event(&make_message(
            "t".repeat(MAX_TITLE_BYTES + 1),
            "ok".to_string()
        ))
        .is_none());
        assert!(parse_event(&make_message(
            "ok".to_string(),
            "m".repeat(MAX_MESSAGE_BYTES + 1)
        ))
        .is_none());
        assert!(parse_event(&make_message("界".repeat(341), "ok".to_string())).is_some());
        assert!(parse_event(&make_message("界".repeat(342), "ok".to_string())).is_none());
    }

    #[test]
    fn malformed_fields_are_dropped_and_missing_message_uses_safe_default() {
        for data in [
            br#"{"event":"message","id":"000000000001","topic":"test-topic","title":7}"#.as_slice(),
            br#"{"event":"message","id":"000000000001","topic":"test-topic","message":false}"#.as_slice(),
            br#"{"event":"message","id":"000000000001","topic":"test-topic","title":"bad\u0000title"}"#.as_slice(),
            br#"{"event":"message""#.as_slice(),
            b"{\xff}".as_slice(),
        ] {
            assert!(parse_event(data).is_none());
        }

        let Some(IncomingEvent::Message(message)) =
            parse_event(br#"{"event":"message","id":"000000000001","topic":"test-topic"}"#)
        else {
            panic!("message without body should use the safe default");
        };
        assert_eq!(message.message, "triggered");
        assert_eq!(message.title, "ntfy 消息");
    }

    #[tokio::test]
    async fn reqwest_source_encodes_since_as_one_query_parameter() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_sender, request_receiver) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let count = stream.read(&mut request).await.unwrap();
            request_sender
                .send(String::from_utf8_lossy(&request[..count]).into_owned())
                .unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        let config = ValidatedSubscriptionConfig::try_from(SubscriptionConfig {
            server: format!("http://{address}/base"),
            username: String::new(),
            password: String::new(),
            topic: "alerts".to_string(),
            allow_insecure_http: false,
        })
        .unwrap();

        let source = ReqwestSource::default();
        drop(
            source
                .connect(config, Some(CURSOR_1.to_string()))
                .await
                .unwrap(),
        );
        let request = request_receiver.await.unwrap();
        assert_eq!(
            request.lines().next(),
            Some("GET /base/alerts/sse?since=000000000001 HTTP/1.1")
        );
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn reqwest_source_rejects_invalid_cursor_before_network() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let config = ValidatedSubscriptionConfig::try_from(SubscriptionConfig {
            server: format!("http://{address}"),
            username: String::new(),
            password: String::new(),
            topic: "alerts".to_string(),
            allow_insecure_http: false,
        })
        .unwrap();

        let source = ReqwestSource::default();
        let result = source.connect(config, Some("all".to_string())).await;
        assert!(result.is_err());
        assert!(
            tokio::time::timeout(Duration::from_millis(150), listener.accept())
                .await
                .is_err(),
            "invalid cursor reached the network"
        );
    }

    #[tokio::test]
    async fn reqwest_source_rejects_redirect_that_injects_replay_query() {
        let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_address = target_listener.local_addr().unwrap();
        let (target_hit_sender, target_hit_receiver) = oneshot::channel();
        let target_task = tokio::spawn(async move {
            let (mut stream, _) = target_listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await;
            let _ = target_hit_sender.send(());
        });

        let redirect_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect_address = redirect_listener.local_addr().unwrap();
        let redirect_task = tokio::spawn(async move {
            let (mut stream, _) = redirect_listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await;
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://{target_address}/alerts/sse?since=all\r\n\
                 Content-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let config = ValidatedSubscriptionConfig::try_from(SubscriptionConfig {
            server: format!("http://{redirect_address}"),
            username: String::new(),
            password: String::new(),
            topic: "alerts".to_string(),
            allow_insecure_http: false,
        })
        .unwrap();
        let source = ReqwestSource::default();
        let result = tokio::time::timeout(Duration::from_secs(2), source.connect(config, None))
            .await
            .expect("redirect validation timed out");

        assert!(result.is_err(), "redirect query injection must be rejected");
        assert!(
            tokio::time::timeout(Duration::from_millis(150), target_hit_receiver)
                .await
                .is_err(),
            "redirect target was contacted before validation"
        );
        redirect_task.await.unwrap();
        target_task.abort();
    }

    #[tokio::test]
    async fn reqwest_source_rejects_redirect_that_drops_cursor() {
        let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_address = target_listener.local_addr().unwrap();
        let (target_hit_sender, target_hit_receiver) = oneshot::channel();
        let target_task = tokio::spawn(async move {
            let (mut stream, _) = target_listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await;
            let _ = target_hit_sender.send(());
            let _ = stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                      Content-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
        });

        let redirect_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect_address = redirect_listener.local_addr().unwrap();
        let redirect_task = tokio::spawn(async move {
            let (mut stream, _) = redirect_listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await;
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://{target_address}/alerts/sse\r\n\
                 Content-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let config = ValidatedSubscriptionConfig::try_from(SubscriptionConfig {
            server: format!("http://{redirect_address}"),
            username: String::new(),
            password: String::new(),
            topic: "alerts".to_string(),
            allow_insecure_http: false,
        })
        .unwrap();
        let source = ReqwestSource::default();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            source.connect(config, Some(CURSOR_1.to_string())),
        )
        .await
        .expect("redirect validation timed out");

        assert!(result.is_err(), "redirect without cursor must be rejected");
        assert!(
            tokio::time::timeout(Duration::from_millis(150), target_hit_receiver)
                .await
                .is_err(),
            "redirect target was contacted before validation"
        );
        redirect_task.await.unwrap();
        target_task.abort();
    }

    #[tokio::test]
    async fn reqwest_source_follows_redirect_that_preserves_cursor() {
        let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_address = target_listener.local_addr().unwrap();
        let (target_request_sender, target_request_receiver) = oneshot::channel();
        let target_task = tokio::spawn(async move {
            let (mut stream, _) = target_listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let count = stream.read(&mut request).await.unwrap();
            target_request_sender
                .send(String::from_utf8_lossy(&request[..count]).into_owned())
                .unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });

        let redirect_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect_address = redirect_listener.local_addr().unwrap();
        let redirect_task = tokio::spawn(async move {
            let (mut stream, _) = redirect_listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await;
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://{target_address}/alerts/sse?since={CURSOR_1}\r\n\
                 Content-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let config = ValidatedSubscriptionConfig::try_from(SubscriptionConfig {
            server: format!("http://{redirect_address}"),
            username: String::new(),
            password: String::new(),
            topic: "alerts".to_string(),
            allow_insecure_http: false,
        })
        .unwrap();
        let source = ReqwestSource::default();
        drop(
            source
                .connect(config, Some(CURSOR_1.to_string()))
                .await
                .unwrap(),
        );

        let target_request = target_request_receiver.await.unwrap();
        assert_eq!(
            target_request.lines().next(),
            Some("GET /alerts/sse?since=000000000001 HTTP/1.1")
        );
        redirect_task.await.unwrap();
        target_task.await.unwrap();
    }

    #[tokio::test]
    async fn sink_failure_does_not_rewind_committed_cursor() {
        let (sender, plan) = channel_plan();
        let source = MockSource::with_plans([plan, ConnectPlan::Pending]);
        let store = Arc::new(MockStore::default());
        let subscription_key = key("https://example.test");
        let controller = SubscriptionController::default();
        let sink = Arc::new(RecordingSink {
            fail_messages: true,
            ..RecordingSink::default()
        });
        let task = spawn_subscription(
            &controller,
            core_with_store(Arc::clone(&source), Arc::clone(&store)),
            config("https://example.test"),
            Arc::clone(&sink),
        );

        send(&sender, "data: {\"event\":\"open\"}\n\n");
        sink.wait_for_state(SubscriptionState::Connected).await;
        send(
            &sender,
            "data: {\"id\":\"000000000001\",\"event\":\"message\",\"topic\":\"test-topic\",\"message\":\"hello\"}\n\n",
        );
        store.wait_for_cursor(&subscription_key, CURSOR_1).await;
        assert_eq!(sink.message_count(), 0);
        assert_eq!(sink.last_state(), Some(SubscriptionState::Connected));

        drop(sender);
        source.wait_for_attempts(2).await;
        assert_eq!(source.attempts()[1].since.as_deref(), Some(CURSOR_1));

        controller.stop();
        task.await.unwrap();
    }
}
