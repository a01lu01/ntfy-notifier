use crate::config::Config;
use crate::endpoint::{self, ValidatedEndpoint};
use futures_util::future::BoxFuture;
use futures_util::{Stream, StreamExt};
use serde::Serialize;
use serde_json::Value;
use std::pin::Pin;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SubscriptionConfig {
    pub server: String,
    pub username: String,
    pub password: String,
    pub topic: String,
    pub allow_insecure_http: bool,
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
        Ok(Self {
            endpoint,
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

pub(crate) trait SubscriptionSink: Send + Sync {
    fn state_changed(&self, state: SubscriptionState);
    fn message_received(&self, message: SubscriptionMessage) -> Result<(), String>;
}

type SubscriptionStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, String>> + Send>>;

pub(crate) trait SubscriptionSource: Send + Sync {
    fn connect(
        &self,
        config: ValidatedSubscriptionConfig,
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
    ) -> BoxFuture<'static, Result<SubscriptionStream, String>> {
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
            let mut request = client
                .get(config.endpoint.subscription)
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
    retry: RetryPolicy,
}

impl Default for SubscriptionCore {
    fn default() -> Self {
        Self {
            source: Arc::new(ReqwestSource::default()),
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
            let connection = tokio::select! {
                _ = context.cancellation.cancelled() => return,
                connection = self.source.connect(config.clone()) => connection,
            };

            let opened = match connection {
                Ok(stream) => self.consume_stream(stream, &context).await,
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

    async fn consume_stream(&self, mut stream: SubscriptionStream, context: &RunContext) -> bool {
        let mut buffer = Vec::new();
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

            buffer.extend_from_slice(&chunk);
            while let Some(position) = buffer.iter().position(|&byte| byte == b'\n') {
                let line: Vec<u8> = buffer.drain(..=position).collect();
                match parse_line(&line) {
                    Some(IncomingEvent::Open) => {
                        if !opened {
                            opened = true;
                            if !context.publish_state(SubscriptionState::Connected).await {
                                return opened;
                            }
                        }
                    }
                    Some(IncomingEvent::Message(message)) => {
                        if let Err(error) = context.deliver_message(message) {
                            eprintln!("[ntfy] 消息处理失败：{error}");
                            return opened;
                        }
                    }
                    None => {}
                }
            }
        }

        opened
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

    fn deliver_message(&self, message: SubscriptionMessage) -> Result<(), String> {
        if self.is_cancelled_or_stale() {
            return Ok(());
        }
        self.sink.message_received(message)
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

fn parse_line(line: &[u8]) -> Option<IncomingEvent> {
    let text = std::str::from_utf8(line).ok()?.trim();
    let data = text.strip_prefix("data:")?.trim_start();
    let value: Value = serde_json::from_str(data).ok()?;
    match value.get("event").and_then(Value::as_str).unwrap_or("") {
        "open" => Some(IncomingEvent::Open),
        "message" => Some(IncomingEvent::Message(SubscriptionMessage {
            id: value
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            topic: value
                .get("topic")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            title: value
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("ntfy 消息")
                .to_string(),
            message: value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or(data)
                .to_string(),
        })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::time::Instant;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Notify;

    enum ConnectPlan {
        Fail(&'static str),
        Pending,
        Stream(mpsc::UnboundedReceiver<Result<Vec<u8>, String>>),
    }

    #[derive(Default)]
    struct MockSource {
        plans: Mutex<VecDeque<ConnectPlan>>,
        attempts: Mutex<Vec<ValidatedSubscriptionConfig>>,
        changed: Notify,
    }

    impl MockSource {
        fn with_plans(plans: impl IntoIterator<Item = ConnectPlan>) -> Arc<Self> {
            Arc::new(Self {
                plans: Mutex::new(plans.into_iter().collect()),
                ..Self::default()
            })
        }

        fn attempts(&self) -> Vec<ValidatedSubscriptionConfig> {
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
        ) -> BoxFuture<'static, Result<SubscriptionStream, String>> {
            self.attempts.lock().unwrap().push(config);
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

    fn channel_plan() -> (mpsc::UnboundedSender<Result<Vec<u8>, String>>, ConnectPlan) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (sender, ConnectPlan::Stream(receiver))
    }

    fn core(source: Arc<MockSource>) -> SubscriptionCore {
        SubscriptionCore {
            source,
            retry: RetryPolicy {
                initial: Duration::from_millis(100),
                maximum: Duration::from_millis(200),
                stream_timeout: Duration::from_secs(2),
            },
        }
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
            "data: {\"id\":\"before-open\",\"event\":\"message\",\"topic\":\"test-topic\",\"message\":\"hello\"}\n\n",
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
                "data: {\"id\":\"after-open\",\"event\":\"message\",",
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
            .position(|event| event == "message:after-open")
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
            source.attempts()[1].endpoint.server.as_str(),
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
            source.attempts()[1].endpoint.server.as_str(),
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
            source.attempts()[0].endpoint.server.as_str(),
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

    #[tokio::test]
    async fn reqwest_source_applies_redirect_validation_before_following() {
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
                "HTTP/1.1 302 Found\r\nLocation: http://user@{target_address}/alerts/sse\r\n\
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
        let result = tokio::time::timeout(Duration::from_secs(2), source.connect(config))
            .await
            .expect("redirect validation timed out");

        assert!(result.is_err(), "redirect with userinfo must be rejected");
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
    async fn sink_failure_drops_stream_and_retries() {
        let (sender, plan) = channel_plan();
        let source = MockSource::with_plans([plan]);
        let controller = SubscriptionController::default();
        let sink = Arc::new(RecordingSink {
            fail_messages: true,
            ..RecordingSink::default()
        });
        let task = spawn_subscription(
            &controller,
            core(source),
            config("https://example.test"),
            Arc::clone(&sink),
        );

        send(&sender, "data: {\"event\":\"open\"}\n\n");
        sink.wait_for_state(SubscriptionState::Connected).await;
        send(
            &sender,
            "data: {\"id\":\"1\",\"event\":\"message\",\"topic\":\"test-topic\",\"message\":\"hello\"}\n\n",
        );
        sink.wait_for_state(SubscriptionState::Retrying).await;
        assert_eq!(sink.last_state(), Some(SubscriptionState::Retrying));

        controller.stop();
        task.await.unwrap();
    }
}
