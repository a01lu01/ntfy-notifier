package app.ntfy.notifier

import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteException
import android.os.SystemClock
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.BufferedInputStream
import java.io.Closeable
import java.io.IOException
import java.io.OutputStreamWriter
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.nio.charset.StandardCharsets
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.CountDownLatch
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.ThreadFactory
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith

/**
 * End-to-end subscriber proof for API 24. It is intentionally opt-in because it requires the
 * x86_64 Rust library to have been copied into jniLibs before Gradle packages the test target.
 * CI must pass -Pandroid.testInstrumentationRunnerArguments.ntfySubscriberIntegration=true.
 */
@RunWith(AndroidJUnit4::class)
class SubscriberServiceIntegrationInstrumentationTest {
  private val instrumentation = InstrumentationRegistry.getInstrumentation()
  private val context: Context = instrumentation.targetContext.applicationContext

  @Test
  fun serviceReceivesAndPersistsLoopbackSseWithoutStartingActivityOrDuplicatingConnection() {
    assertTrue(
      "CI must pass ntfySubscriberIntegration=true; this test must never silently skip",
      InstrumentationRegistry.getArguments()
        .getString(ARG_INTEGRATION)
        .equals("true", ignoreCase = true)
    )
    assertTrue(
      "ntfy_notifier_lib must load; CI must package the x86_64 Rust library",
      NativeSubscriber.isAvailable()
    )

    val originalConfig = ConfigStore(context).loadPublicConfig()
    val topic = "ci_${UUID.randomUUID().toString().replace("-", "").take(16)}"
    val messageId = UUID.randomUUID().toString().replace("-", "").take(12)
    val title = "API 24 subscriber"
    val body = "independent foreground service persisted this message"
    val server = LoopbackSseServer(topic, messageId, title, body)

    try {
      ConfigStore(context).savePublicConfig(
        PublicConfig(
          server = server.baseUrl,
          username = "",
          password = "",
          topic = topic,
          themeMode = "system",
          autoStart = false,
          autoCopyOtp = false,
          allowInsecureHttp = false
        )
      )

      // This is the only component started by the test; no Activity or Tauri runtime is needed.
      NotificationService.sendAction(context, NotificationService.ACTION_START)

      assertTrue("subscriber never opened the loopback SSE stream", server.awaitConnection(15))
      waitUntil(20_000, "subscriber did not persist the ntfy message") {
        readHistoryMessage(messageId) == (title to body)
      }

      repeat(4) {
        NotificationService.sendAction(context, NotificationService.ACTION_RECONFIGURE)
      }
      SystemClock.sleep(1_500)

      assertEquals("identical reconfigure actions must not reconnect", 1, server.acceptedCount)
      assertEquals("the application must have exactly one active SSE", 1, server.maximumActive)
      assertEquals("/$topic/sse", server.requestPaths.single())
      assertEquals(title to body, readHistoryMessage(messageId))
    } finally {
      try {
        NotificationService.sendAction(context, NotificationService.ACTION_STOP)
        waitUntil(5_000, "subscriber did not close its SSE connection") {
          server.activeCount == 0
        }
      } finally {
        server.close()
        ConfigStore(context).savePublicConfig(originalConfig)
      }
    }
  }

  private fun readHistoryMessage(id: String): Pair<String, String>? {
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
      // The service may be between database creation and its first committed transaction.
      null
    }
  }

  private fun waitUntil(timeoutMillis: Long, failureMessage: String, condition: () -> Boolean) {
    val deadline = SystemClock.elapsedRealtime() + timeoutMillis
    while (SystemClock.elapsedRealtime() < deadline) {
      if (condition()) return
      SystemClock.sleep(100)
    }
    fail(failureMessage)
  }

  private class LoopbackSseServer(
    topic: String,
    messageId: String,
    title: String,
    body: String
  ) : Closeable {
    private val running = AtomicBoolean(true)
    private val accepted = AtomicInteger(0)
    private val active = AtomicInteger(0)
    private val maximum = AtomicInteger(0)
    private val firstConnection = CountDownLatch(1)
    private val sockets = ConcurrentHashMap.newKeySet<Socket>()
    private val acceptExecutor: ExecutorService = Executors.newSingleThreadExecutor(
      namedDaemonThreads("ntfy-test-accept")
    )
    private val connectionExecutor: ExecutorService = Executors.newCachedThreadPool(
      namedDaemonThreads("ntfy-test-connection")
    )
    private val serverSocket = ServerSocket(
      0,
      16,
      InetAddress.getByName("127.0.0.1")
    )
    private val openEvent = JSONObject()
      .put("event", "open")
      .put("topic", topic)
      .toString()
    private val messageEvent = JSONObject()
      .put("event", "message")
      .put("id", messageId)
      .put("topic", topic)
      .put("title", title)
      .put("message", body)
      .toString()

    val requestPaths = ConcurrentLinkedQueue<String>()
    val baseUrl: String = "http://127.0.0.1:${serverSocket.localPort}"
    val acceptedCount: Int get() = accepted.get()
    val activeCount: Int get() = active.get()
    val maximumActive: Int get() = maximum.get()

    init {
      acceptExecutor.execute {
        while (running.get()) {
          try {
            val socket = serverSocket.accept()
            accepted.incrementAndGet()
            sockets.add(socket)
            connectionExecutor.execute { serve(socket) }
          } catch (_: IOException) {
            if (running.get()) throw AssertionError("loopback SSE accept failed")
          }
        }
      }
    }

    fun awaitConnection(timeoutSeconds: Long): Boolean {
      return firstConnection.await(timeoutSeconds, TimeUnit.SECONDS)
    }

    private fun serve(socket: Socket) {
      try {
        socket.tcpNoDelay = true
        val input = BufferedInputStream(socket.getInputStream())
        val requestLine = readAsciiLine(input) ?: return
        requestPaths.add(requestLine.split(' ').getOrNull(1).orEmpty())
        var headerBytes = requestLine.length
        while (true) {
          val line = readAsciiLine(input) ?: return
          headerBytes += line.length
          if (headerBytes > MAX_REQUEST_HEADER_BYTES) return
          if (line.isEmpty()) break
        }

        val activeNow = active.incrementAndGet()
        maximum.updateAndGet { prior -> maxOf(prior, activeNow) }
        firstConnection.countDown()
        try {
          val writer = OutputStreamWriter(socket.getOutputStream(), StandardCharsets.UTF_8)
          writer.write("HTTP/1.1 200 OK\r\n")
          writer.write("Content-Type: text/event-stream\r\n")
          writer.write("Cache-Control: no-cache\r\n")
          writer.write("Transfer-Encoding: chunked\r\n")
          writer.write("Connection: keep-alive\r\n\r\n")
          writeChunk(writer, "data: $openEvent\n\n")
          writeChunk(writer, "data: $messageEvent\n\n")
          writer.flush()
          while (running.get() && !socket.isClosed) {
            Thread.sleep(250)
            writeChunk(writer, ": keep-alive\n\n")
            writer.flush()
          }
        } finally {
          active.decrementAndGet()
        }
      } catch (_: IOException) {
        // Closing the service or server is the expected end of an SSE stream.
      } catch (_: InterruptedException) {
        Thread.currentThread().interrupt()
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
      return null
    }

    private fun writeChunk(writer: OutputStreamWriter, payload: String) {
      val byteLength = payload.toByteArray(StandardCharsets.UTF_8).size
      writer.write(byteLength.toString(16))
      writer.write("\r\n")
      writer.write(payload)
      writer.write("\r\n")
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

      private fun namedDaemonThreads(prefix: String): ThreadFactory {
        val sequence = AtomicInteger(0)
        return ThreadFactory { task ->
          Thread(task, "$prefix-${sequence.incrementAndGet()}").apply { isDaemon = true }
        }
      }
    }
  }

  companion object {
    private const val ARG_INTEGRATION = "ntfySubscriberIntegration"
  }
}
