package app.ntfy.notifier

import android.app.Activity
import android.app.ActivityManager
import android.app.Application
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteException
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import androidx.test.platform.app.InstrumentationRegistry
import com.why.ntfy_notifier.MainActivity
import java.io.BufferedInputStream
import java.io.Closeable
import java.io.IOException
import java.io.OutputStreamWriter
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.nio.charset.StandardCharsets
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.ThreadFactory
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import org.json.JSONObject
import org.junit.Assert.assertTrue
import org.junit.Assert.fail

internal const val ARG_SUBSCRIBER_INTEGRATION = "ntfySubscriberIntegration"

internal fun requireSubscriberIntegration() {
  assertTrue(
    "subscriber integration scenarios must run on the required API 24 emulator",
    Build.VERSION.SDK_INT == Build.VERSION_CODES.N
  )
  assertTrue(
    "CI must pass ntfySubscriberIntegration=true; subscriber scenarios must never skip",
    InstrumentationRegistry.getArguments()
      .getString(ARG_SUBSCRIBER_INTEGRATION)
      .equals("true", ignoreCase = true)
  )
  assertTrue(
    "ntfy_notifier_lib must load; CI must package the x86_64 Rust library",
    NativeSubscriber.isAvailable()
  )
}

internal fun waitUntil(
  timeoutMillis: Long,
  failureMessage: String,
  condition: () -> Boolean
) {
  val deadline = SystemClock.elapsedRealtime() + timeoutMillis
  while (SystemClock.elapsedRealtime() < deadline) {
    if (condition()) return
    SystemClock.sleep(100)
  }
  fail(failureMessage)
}

internal fun readHistoryMessage(context: Context, id: String): Pair<String, String>? {
  val databaseFile = context.dataDir.resolve("history.db")
  if (!databaseFile.isFile) return null
  return try {
    SQLiteDatabase.openDatabase(
      databaseFile.absolutePath,
      null,
      SQLiteDatabase.OPEN_READONLY
    ).use { database ->
      database.rawQuery(
        "SELECT title, message FROM messages WHERE id = ?",
        arrayOf(id)
      ).use { cursor ->
        if (!cursor.moveToFirst()) null else cursor.getString(0) to cursor.getString(1)
      }
    }
  } catch (_: SQLiteException) {
    // The subscriber may be creating the WAL or committing the message transaction.
    null
  }
}

internal fun readSubscriptionCursor(context: Context, topic: String): String? {
  val databaseFile = context.dataDir.resolve("history.db")
  if (!databaseFile.isFile) return null
  return try {
    SQLiteDatabase.openDatabase(
      databaseFile.absolutePath,
      null,
      SQLiteDatabase.OPEN_READONLY
    ).use { database ->
      database.rawQuery(
        "SELECT last_id FROM subscription_cursors WHERE topic = ?",
        arrayOf(topic)
      ).use { cursor ->
        if (!cursor.moveToFirst()) null else cursor.getString(0)
      }
    }
  } catch (_: SQLiteException) {
    null
  }
}

internal fun subscriberProcessIds(context: Context): Set<Int> {
  val processName = "${context.packageName}:subscriber"
  val manager = context.getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager
  return manager.runningAppProcesses.orEmpty()
    .asSequence()
    .filter { it.processName == processName }
    .map { it.pid }
    .toSet()
}

internal fun stopSubscriber(context: Context, server: LoopbackSseServer? = null) {
  val observableRunning =
    (server?.activeCount ?: 0) > 0 || hasSubscriberForegroundNotification(context)
  NotificationService.sendAction(context, NotificationService.ACTION_STOP)
  if (observableRunning) {
    waitUntil(10_000, "subscriber did not stop or remove its foreground notification") {
      server?.assertHealthy()
      (server?.activeCount ?: 0) == 0 && !hasSubscriberForegroundNotification(context)
    }
  } else {
    // STOP is serialized with subscriber startup; give the process executor time to drain before
    // a test overwrites the shared configuration.
    SystemClock.sleep(750)
  }
}

private fun hasSubscriberForegroundNotification(context: Context): Boolean {
  val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
  return manager.activeNotifications.any { notification ->
    notification.id == NotificationService.NOTIFICATION_ID_LATEST
  }
}

internal fun launchAndFinishMainActivity(): Activity {
  val instrumentation = InstrumentationRegistry.getInstrumentation()
  val context = instrumentation.targetContext
  val activity = instrumentation.startActivitySync(
    Intent(context, MainActivity::class.java)
      .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
  )
  instrumentation.waitForIdleSync()
  instrumentation.runOnMainSync { activity.finish() }
  instrumentation.waitForIdleSync()
  waitUntil(10_000, "MainActivity did not finish") {
    activity.isDestroyed
  }
  return activity
}

internal class MainActivityProbe(context: Context) :
  Application.ActivityLifecycleCallbacks,
  Closeable {
  private val application = context.applicationContext as Application
  private val createdCount = AtomicInteger(0)

  val creations: Int get() = createdCount.get()

  init {
    application.registerActivityLifecycleCallbacks(this)
  }

  override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) {
    if (activity is MainActivity) createdCount.incrementAndGet()
  }

  override fun onActivityStarted(activity: Activity) = Unit

  override fun onActivityResumed(activity: Activity) = Unit

  override fun onActivityPaused(activity: Activity) = Unit

  override fun onActivityStopped(activity: Activity) = Unit

  override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit

  override fun onActivityDestroyed(activity: Activity) = Unit

  override fun close() {
    application.unregisterActivityLifecycleCallbacks(this)
  }
}

internal data class LoopbackMessage(
  val id: String,
  val title: String,
  val body: String
)

/**
 * Minimal ntfy-compatible HTTP/1.1 SSE endpoint. Each accepted connection receives the script at
 * the matching index, then remains open so tests can detect overlapping subscriptions.
 */
internal class LoopbackSseServer(
  private val topic: String,
  private val connectionScripts: List<List<LoopbackMessage>>
) : Closeable {
  private val running = AtomicBoolean(true)
  private val accepted = AtomicInteger(0)
  private val active = AtomicInteger(0)
  private val maximum = AtomicInteger(0)
  private val failure = AtomicReference<Throwable?>(null)
  private val sockets = ConcurrentHashMap.newKeySet<Socket>()
  private val acceptExecutor: ExecutorService = Executors.newSingleThreadExecutor(
    namedDaemonThreads("ntfy-test-accept")
  )
  private val connectionExecutor: ExecutorService = Executors.newCachedThreadPool(
    namedDaemonThreads("ntfy-test-connection")
  )
  private val serverSocket = ServerSocket(0, 16, InetAddress.getByName("127.0.0.1"))
  private val openEvent = JSONObject()
    .put("event", "open")
    .put("topic", topic)
    .toString()

  val requestPaths = ConcurrentLinkedQueue<String>()
  val baseUrl: String = "http://127.0.0.1:${serverSocket.localPort}"
  val acceptedCount: Int get() = accepted.get()
  val activeCount: Int get() = active.get()
  val maximumActive: Int get() = maximum.get()

  constructor(topic: String, message: LoopbackMessage) : this(
    topic,
    listOf(listOf(message))
  )

  init {
    acceptExecutor.execute {
      while (running.get()) {
        try {
          val socket = serverSocket.accept()
          val connectionIndex = accepted.getAndIncrement()
          sockets.add(socket)
          connectionExecutor.execute { serve(socket, connectionIndex) }
        } catch (error: IOException) {
          if (running.get()) recordFailure(error)
        } catch (error: RuntimeException) {
          if (running.get()) recordFailure(error)
        }
      }
    }
  }

  fun awaitConnections(expected: Int, timeoutMillis: Long = 20_000) {
    waitUntil(timeoutMillis, "subscriber did not open $expected loopback SSE connection(s)") {
      assertHealthy()
      acceptedCount >= expected
    }
  }

  fun assertHealthy() {
    failure.get()?.let { error ->
      throw AssertionError("loopback SSE server failed", error)
    }
  }

  private fun serve(socket: Socket, connectionIndex: Int) {
    try {
      socket.tcpNoDelay = true
      val input = BufferedInputStream(socket.getInputStream())
      val requestLine = readAsciiLine(input) ?: return
      requestPaths.add(requestLine.split(' ').getOrNull(1).orEmpty())
      var headerBytes = requestLine.length
      while (true) {
        val line = readAsciiLine(input) ?: return
        headerBytes += line.length
        if (headerBytes > MAX_REQUEST_HEADER_BYTES) {
          throw IOException("request headers exceeded test-server limit")
        }
        if (line.isEmpty()) break
      }

      val activeNow = active.incrementAndGet()
      maximum.updateAndGet { prior -> maxOf(prior, activeNow) }
      try {
        val writer = OutputStreamWriter(socket.getOutputStream(), StandardCharsets.UTF_8)
        writer.write("HTTP/1.1 200 OK\r\n")
        writer.write("Content-Type: text/event-stream\r\n")
        writer.write("Cache-Control: no-cache\r\n")
        writer.write("Transfer-Encoding: chunked\r\n")
        writer.write("Connection: keep-alive\r\n\r\n")
        writeChunk(writer, "data: $openEvent\n\n")
        connectionScripts.getOrNull(connectionIndex).orEmpty().forEach { message ->
          val event = JSONObject()
            .put("event", "message")
            .put("id", message.id)
            .put("topic", topic)
            .put("title", message.title)
            .put("message", message.body)
            .toString()
          writeChunk(writer, "data: $event\n\n")
        }
        writer.flush()
        while (running.get() && !socket.isClosed) {
          Thread.sleep(KEEPALIVE_MILLIS)
          writeChunk(writer, ": keep-alive\n\n")
          writer.flush()
        }
      } finally {
        active.decrementAndGet()
      }
    } catch (_: IOException) {
      // Service stop, process death, and server close terminate an SSE stream this way.
    } catch (_: InterruptedException) {
      Thread.currentThread().interrupt()
    } catch (error: RuntimeException) {
      if (running.get()) recordFailure(error)
    } finally {
      sockets.remove(socket)
      try {
        socket.close()
      } catch (_: IOException) {
        // Already closed while shutting down the test server.
      }
    }
  }

  private fun readAsciiLine(input: BufferedInputStream): String? {
    val bytes = ArrayList<Byte>()
    while (bytes.size <= MAX_REQUEST_HEADER_BYTES) {
      val next = input.read()
      if (next == -1) return null
      if (next == '\n'.code) {
        if (bytes.lastOrNull() == '\r'.code.toByte()) bytes.removeAt(bytes.lastIndex)
        return bytes.toByteArray().toString(StandardCharsets.US_ASCII)
      }
      bytes.add(next.toByte())
    }
    throw IOException("request line exceeded test-server limit")
  }

  private fun writeChunk(writer: OutputStreamWriter, payload: String) {
    val byteLength = payload.toByteArray(StandardCharsets.UTF_8).size
    writer.write(byteLength.toString(16))
    writer.write("\r\n")
    writer.write(payload)
    writer.write("\r\n")
  }

  private fun recordFailure(error: Throwable) {
    failure.compareAndSet(null, error)
  }

  override fun close() {
    if (!running.compareAndSet(true, false)) return
    try {
      serverSocket.close()
    } catch (_: IOException) {
      // Already closed.
    }
    sockets.forEach { socket ->
      try {
        socket.close()
      } catch (_: IOException) {
        // Already closed.
      }
    }
    acceptExecutor.shutdownNow()
    connectionExecutor.shutdownNow()
  }

  companion object {
    private const val MAX_REQUEST_HEADER_BYTES = 16 * 1024
    private const val KEEPALIVE_MILLIS = 50L

    private fun namedDaemonThreads(prefix: String): ThreadFactory {
      val sequence = AtomicInteger(0)
      return ThreadFactory { task ->
        Thread(task, "$prefix-${sequence.incrementAndGet()}").apply { isDaemon = true }
      }
    }
  }
}
