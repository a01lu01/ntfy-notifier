package app.ntfy.notifier

import android.content.Context
import android.os.SystemClock
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.util.UUID
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

/** End-to-end API 24 proof that the foreground-service process owns the sole SSE connection. */
@RunWith(AndroidJUnit4::class)
class SubscriberServiceIntegrationInstrumentationTest {
  private val context: Context =
    InstrumentationRegistry.getInstrumentation().targetContext.applicationContext

  @Test
  fun serviceReceivesAndPersistsLoopbackSseWithoutStartingActivityOrDuplicatingConnection() {
    requireSubscriberIntegration()
    stopSubscriber(context)

    val originalConfig = ConfigStore(context).loadPublicConfig()
    val topic = "ci_${UUID.randomUUID().toString().replace("-", "").take(16)}"
    val message = LoopbackMessage(
      id = UUID.randomUUID().toString().replace("-", "").take(12),
      title = "API 24 subscriber",
      body = "independent foreground service persisted this message"
    )
    val server = LoopbackSseServer(topic, message)

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

      server.awaitConnections(1, 15_000)
      waitUntil(20_000, "subscriber did not persist the ntfy message") {
        server.assertHealthy()
        readHistoryMessage(context, message.id) == (message.title to message.body)
      }

      repeat(4) {
        NotificationService.sendAction(context, NotificationService.ACTION_RECONFIGURE)
      }
      SystemClock.sleep(1_500)
      server.assertHealthy()

      assertEquals("identical reconfigure actions must not reconnect", 1, server.acceptedCount)
      assertEquals("the application must have exactly one active SSE", 1, server.maximumActive)
      assertEquals("/$topic/sse", server.requestPaths.single())
      assertEquals(message.title to message.body, readHistoryMessage(context, message.id))
    } finally {
      try {
        stopSubscriber(context, server)
      } finally {
        server.close()
        ConfigStore(context).savePublicConfig(originalConfig)
      }
    }
  }
}
