package app.ntfy.notifier

import androidx.annotation.Keep
import java.lang.ref.WeakReference
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicLong

/**
 * JNI entry points for the subscriber process. A loader failure is captured so an incompatible
 * native library becomes a visible configuration_error instead of a service crash loop.
 */
@Keep
object NativeSubscriber {
  private val libraryLoaded = try {
    System.loadLibrary("ntfy_notifier_lib")
    true
  } catch (_: LinkageError) {
    false
  } catch (_: SecurityException) {
    false
  }

  fun isAvailable(): Boolean = libraryLoaded

  @Keep
  @JvmStatic
  external fun nativeStart(
    dataDir: String,
    session: Long,
    server: String,
    username: String,
    password: String,
    topic: String,
    allowInsecureHttp: Boolean,
    callback: SubscriberCallback
  ): Boolean

  @Keep
  @JvmStatic
  external fun nativeReconfigure(
    dataDir: String,
    session: Long,
    server: String,
    username: String,
    password: String,
    topic: String,
    allowInsecureHttp: Boolean,
    callback: SubscriberCallback
  ): Boolean

  @Keep
  @JvmStatic
  external fun nativeStop()
}

internal interface SubscriberEventSink {
  fun onSubscriberState(session: Long, state: String)

  fun onSubscriberMessage(session: Long, title: String, message: String, otp: String?)
}

/** Kept by name because Rust resolves these methods with GetMethodID. */
@Keep
class SubscriberCallback internal constructor() {
  @Keep
  fun onNativeState(session: Long, state: String) {
    SubscriberProcessControl.routeState(session, state)
  }

  @Keep
  fun onNativeMessage(session: Long, title: String, message: String, otp: String?) {
    SubscriberProcessControl.routeMessage(session, title, message, otp)
  }
}

/** Monotonic process-local sessions make callbacks from cancelled native generations harmless. */
internal class SubscriberSessionTracker {
  private val sequence = AtomicLong(0)
  private val current = AtomicLong(0)

  fun advance(): Long {
    val session = sequence.incrementAndGet()
    current.set(session)
    return session
  }

  fun isCurrent(session: Long): Boolean = session != 0L && current.get() == session

  /** True for every non-zero session allocated by this process, including superseded sessions. */
  fun hasIssued(session: Long): Boolean {
    return session > 0L && session <= sequence.get()
  }
}

/**
 * Rust's engine also lives for the lifetime of :subscriber, not a Service instance. Keeping the
 * Kotlin sequence at the same process lifetime prevents a recreated Service from reusing session
 * 1 while the native engine still remembers a larger session.
 */
internal object SubscriberProcessSession {
  val tracker = SubscriberSessionTracker()
}

/** Thread-safe owner leases, separated from the Android singleton so lifecycle races are testable. */
internal class SubscriberOwnerLeaseRegistry {
  private val lock = Any()
  private var sequence = 0L
  private var ownerLease = 0L
  private var ownerSink: WeakReference<SubscriberEventSink>? = null

  fun claim(sink: SubscriberEventSink): Long = synchronized(lock) {
    val lease = sequence + 1L
    check(lease > 0L) { "subscriber owner lease exhausted" }
    sequence = lease
    ownerLease = lease
    ownerSink = WeakReference(sink)
    lease
  }

  fun isOwner(lease: Long): Boolean = synchronized(lock) {
    lease != 0L && ownerLease == lease
  }

  fun release(lease: Long): Boolean = synchronized(lock) {
    if (lease == 0L || ownerLease != lease) {
      false
    } else {
      ownerLease = 0L
      ownerSink = null
      true
    }
  }

  fun isUnowned(): Boolean = synchronized(lock) {
    ownerLease == 0L
  }

  fun currentSink(): SubscriberEventSink? = synchronized(lock) {
    ownerSink?.get()
  }
}

/**
 * JNI control is process-global, so lifecycle operations from consecutive Service instances must
 * share one queue. Otherwise an old instance's asynchronous stop can race after the replacement
 * instance's start and silently tear down the new subscription.
 */
internal object SubscriberProcessControl {
  private val owners = SubscriberOwnerLeaseRegistry()

  val executor: ExecutorService = Executors.newSingleThreadExecutor { task ->
    Thread(task, "ntfy-subscriber-control").apply { isDaemon = true }
  }

  val callback = SubscriberCallback()

  fun claim(sink: SubscriberEventSink): Long = owners.claim(sink)

  fun isOwner(lease: Long): Boolean = owners.isOwner(lease)

  fun release(lease: Long): Boolean = owners.release(lease)

  fun isUnowned(): Boolean = owners.isUnowned()

  fun routeState(session: Long, state: String) {
    owners.currentSink()?.onSubscriberState(session, state)
  }

  fun routeMessage(session: Long, title: String, message: String, otp: String?) {
    if (!SubscriberProcessSession.tracker.hasIssued(session)) return
    owners.currentSink()?.onSubscriberMessage(session, title, message, otp)
  }
}

/** Only fields that change the SSE connection participate in reconnect decisions. */
internal data class NativeSubscriptionConfig(
  val server: String,
  val username: String,
  val password: String,
  val topic: String,
  val allowInsecureHttp: Boolean
) {
  override fun toString(): String = "NativeSubscriptionConfig(<redacted>)"
}

internal fun SubscriberConfig.toNativeSubscriptionConfig() = NativeSubscriptionConfig(
  server = server,
  username = username,
  password = password,
  topic = topic,
  allowInsecureHttp = allowInsecureHttp
)
