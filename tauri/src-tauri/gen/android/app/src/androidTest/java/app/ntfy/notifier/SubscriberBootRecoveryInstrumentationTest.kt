package app.ntfy.notifier

import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.os.Process
import android.os.SystemClock
import android.util.Base64
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.RandomAccessFile
import java.util.UUID
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith

/** API 24 automatic scenarios for boot restore, sticky process recovery, and single ownership. */
@RunWith(AndroidJUnit4::class)
class SubscriberBootRecoveryInstrumentationTest {
  private val context: Context =
    InstrumentationRegistry.getInstrumentation().targetContext.applicationContext

  @Test
  fun disabledAutoStartBootDoesNotConnectOrLaunchMainActivity() {
    requireSubscriberIntegration()
    stopSubscriber(context)

    val originalConfig = ConfigStore(context).loadPublicConfig()
    val topic = randomTopic()
    val server = LoopbackSseServer(
      topic,
      LoopbackMessage(randomMessageId(), "disabled boot", "must not be delivered")
    )
    val activityProbe = MainActivityProbe(context)

    try {
      ConfigStore(context).savePublicConfig(testConfig(server, topic, autoStart = false))

      simulateBootCompleted()
      // The receiver deliberately reads policy off-thread. No service callback exists in the
      // disabled path, so a bounded quiet period is the externally observable assertion.
      SystemClock.sleep(3_000)
      server.assertHealthy()

      assertEquals("auto_start=false must not subscribe after boot", 0, server.acceptedCount)
      assertEquals("boot must never launch MainActivity", 0, activityProbe.creations)
    } finally {
      try {
        stopSubscriber(context, server)
      } finally {
        activityProbe.close()
        server.close()
        ConfigStore(context).savePublicConfig(originalConfig)
      }
    }
  }

  @Test
  fun enabledBootPersistsAndRepeatedBootReconfigureAndActivityDoNotDuplicateSse() {
    requireSubscriberIntegration()
    stopSubscriber(context)

    val originalConfig = ConfigStore(context).loadPublicConfig()
    val topic = randomTopic()
    val message = LoopbackMessage(
      randomMessageId(),
      "boot subscriber",
      "service received this without launching the activity"
    )
    val server = LoopbackSseServer(topic, message)
    val activityProbe = MainActivityProbe(context)

    try {
      ConfigStore(context).savePublicConfig(testConfig(server, topic, autoStart = true))

      simulateBootCompleted()
      server.awaitConnections(1, 20_000)
      waitUntil(20_000, "boot-restored subscriber did not persist its message") {
        server.assertHealthy()
        readHistoryMessage(context, message.id) == (message.title to message.body)
      }
      assertEquals("boot must not launch MainActivity", 0, activityProbe.creations)

      repeat(3) { simulateBootCompleted() }
      repeat(3) {
        NotificationService.sendAction(context, NotificationService.ACTION_RECONFIGURE)
      }
      // The instrumentation runner shares the Tauri host process. Destroying its last Wry
      // Activity intentionally terminates that process, so exercise the user-visible
      // foreground/background cycle instead. MainActivity is singleTask and must be reused.
      repeat(2) { index ->
        launchAndBackgroundMainActivity(
          activityProbe,
          expectedResumeCount = index + 1,
          expectedStopCount = index + 1
        )
      }

      // Give BootReceiver's serial executor and Tauri setup enough time to issue their idempotent
      // control actions before checking the steady state.
      SystemClock.sleep(3_000)
      server.assertHealthy()

      assertEquals("singleTask must reuse the deliberate Activity launch", 1, activityProbe.creations)
      assertEquals("both deliberate Activity launches must resume", 2, activityProbe.resumptions)
      assertEquals("both deliberate Activity launches must return to background", 2, activityProbe.stops)
      assertEquals("repeated controls must keep the original SSE", 1, server.acceptedCount)
      assertEquals("no control path may overlap SSE connections", 1, server.maximumActive)
      assertEquals("/$topic/sse", server.requestPaths.single())
    } finally {
      try {
        stopSubscriber(context, server)
      } finally {
        activityProbe.close()
        server.close()
        ConfigStore(context).savePublicConfig(originalConfig)
      }
    }
  }

  @Test
  fun enabledBootWithEmptyTopicShowsConfigurationErrorWithoutConnectingOrLaunchingActivity() {
    requireSubscriberIntegration()
    stopSubscriber(context)

    val originalConfig = ConfigStore(context).loadPublicConfig()
    val serverTopic = randomTopic()
    val server = LoopbackSseServer(
      serverTopic,
      LoopbackMessage(randomMessageId(), "invalid configuration", "must not be delivered")
    )
    val activityProbe = MainActivityProbe(context)

    try {
      ConfigStore(context).savePublicConfig(
        testConfig(server, topic = "", autoStart = true)
      )

      simulateBootCompleted()
      waitUntil(20_000, "invalid boot configuration did not publish its error state") {
        foregroundStatus() == configurationErrorStatus()
      }
      SystemClock.sleep(1_000)
      server.assertHealthy()

      assertEquals("invalid configuration must never open SSE", 0, server.acceptedCount)
      assertEquals("invalid configuration must never report an active SSE", 0, server.activeCount)
      assertEquals("boot configuration errors must not launch MainActivity", 0, activityProbe.creations)
      assertFalse(
        "configuration errors must not masquerade as connected",
        foregroundStatus()?.subText == "正在接收推送"
      )
    } finally {
      try {
        stopSubscriber(context, server)
      } finally {
        activityProbe.close()
        server.close()
        ConfigStore(context).savePublicConfig(originalConfig)
      }
    }
  }

  @Test
  fun enabledBootWithCorruptedCredentialShowsConfigurationErrorWithoutConnectingOrActivity() {
    requireSubscriberIntegration()
    stopSubscriber(context)

    val originalConfig = ConfigStore(context).loadPublicConfig()
    val topic = randomTopic()
    val server = LoopbackSseServer(
      topic,
      LoopbackMessage(randomMessageId(), "corrupt credential", "must not be delivered")
    )
    val activityProbe = MainActivityProbe(context)

    try {
      ConfigStore(context).savePublicConfig(
        testConfig(server, topic, autoStart = true).copy(
          username = "integration-user",
          password = "integration-secret"
        )
      )
      corruptStoredCredentialAuthenticationTag()

      assertTrue(
        "boot policy must remain readable without decrypting credentials",
        ConfigStore(context).loadSubscriberAutoStartPolicy()
      )
      try {
        ConfigStore(context).loadSubscriberConfig()
        fail("tampered credential unexpectedly decrypted")
      } catch (error: ConfigStoreException) {
        assertEquals(ConfigStoreError.CREDENTIAL_DECRYPT, error.code)
      }

      simulateBootCompleted()
      waitUntil(20_000, "credential failure did not publish its configuration error") {
        foregroundStatus() == configurationErrorStatus()
      }
      SystemClock.sleep(1_000)
      server.assertHealthy()

      assertEquals("unreadable credentials must never open SSE", 0, server.acceptedCount)
      assertEquals("unreadable credentials must never report an active SSE", 0, server.activeCount)
      assertEquals("credential failures at boot must not launch MainActivity", 0, activityProbe.creations)
      assertFalse(
        "credential failures must not masquerade as connected",
        foregroundStatus()?.subText == "正在接收推送"
      )
    } finally {
      try {
        stopSubscriber(context, server)
      } finally {
        activityProbe.close()
        server.close()
        ConfigStore(context).savePublicConfig(originalConfig)
      }
    }
  }

  @Test
  fun stickyProcessDeathRestartsWithCommittedCursorAndOneConnection() {
    requireSubscriberIntegration()
    stopSubscriber(context)

    val originalConfig = ConfigStore(context).loadPublicConfig()
    val topic = randomTopic()
    val first = LoopbackMessage(
      randomMessageId(),
      "before process death",
      "cursor must commit before the subscriber process is killed"
    )
    val second = LoopbackMessage(
      randomMessageId(),
      "after process death",
      "sticky process restart resumed from the committed cursor"
    )
    val server = LoopbackSseServer(
      topic,
      listOf(listOf(first), listOf(second))
    )

    try {
      ConfigStore(context).savePublicConfig(testConfig(server, topic, autoStart = true))
      simulateBootCompleted()

      server.awaitConnections(1, 20_000)
      waitUntil(20_000, "first message and cursor were not committed") {
        server.assertHealthy()
        readHistoryMessage(context, first.id) == (first.title to first.body) &&
          readSubscriptionCursor(context, topic) == first.id
      }
      // The boot request first returns NOT_STICKY, then promotes itself with its one-shot
      // ACTION_ACTIVATE_STICKY token after native startup. Wait past that control handoff before
      // killing the process so recovery proves the complete boot-to-sticky chain.
      SystemClock.sleep(1_000)

      waitUntil(10_000, "dedicated subscriber process did not appear") {
        subscriberProcessIds(context).size == 1
      }
      val originalPid = subscriberProcessIds(context).single()
      assertNotEquals("test must never kill its own instrumentation process", Process.myPid(), originalPid)

      // Kill only package:subscriber. A package force-stop would invalidate START_STICKY and would
      // not exercise Android's cold service reconstruction contract.
      Process.killProcess(originalPid)

      waitUntil(20_000, "killed subscriber PID did not disappear") {
        originalPid !in subscriberProcessIds(context)
      }
      server.awaitConnections(2, 45_000)
      waitUntil(20_000, "START_STICKY did not create a replacement subscriber PID") {
        val pids = subscriberProcessIds(context)
        pids.size == 1 && originalPid !in pids
      }
      waitUntil(25_000, "restarted subscriber did not persist the second message and cursor") {
        server.assertHealthy()
        readHistoryMessage(context, second.id) == (second.title to second.body) &&
          readSubscriptionCursor(context, topic) == second.id
      }

      SystemClock.sleep(1_000)
      server.assertHealthy()
      assertEquals("sticky recovery must open exactly one replacement SSE", 2, server.acceptedCount)
      assertEquals("process replacement must never overlap SSE connections", 1, server.maximumActive)
      assertEquals(
        listOf("/$topic/sse", "/$topic/sse?since=${first.id}"),
        server.requestPaths.toList()
      )
      assertEquals(first.title to first.body, readHistoryMessage(context, first.id))
      assertEquals(second.title to second.body, readHistoryMessage(context, second.id))
    } finally {
      try {
        stopSubscriber(context, server)
      } finally {
        server.close()
        ConfigStore(context).savePublicConfig(originalConfig)
      }
    }
  }

  private fun simulateBootCompleted() {
    val intent = Intent(Intent.ACTION_BOOT_COMPLETED)
    assertTrue(intent.extras == null || intent.extras!!.isEmpty)
    BootReceiver().onReceive(context, intent)
  }

  private fun foregroundStatus(): ForegroundStatus? {
    val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
    val notification = manager.activeNotifications
      .firstOrNull { it.id == NotificationService.NOTIFICATION_ID_LATEST }
      ?.notification
      ?: return null
    return ForegroundStatus(
      title = notification.extras.getCharSequence(Notification.EXTRA_TITLE)?.toString(),
      text = notification.extras.getCharSequence(Notification.EXTRA_TEXT)?.toString(),
      subText = notification.extras.getCharSequence(Notification.EXTRA_SUB_TEXT)?.toString()
    )
  }

  private fun configurationErrorStatus(): ForegroundStatus {
    return ForegroundStatus(
      title = "ntfy-Notifier 配置错误",
      text = "配置不可用，请打开应用检查设置",
      subText = "配置不可用，请打开应用检查设置"
    )
  }

  private fun corruptStoredCredentialAuthenticationTag() {
    val configFile = context.dataDir.resolve("config.json")
    RandomAccessFile(context.dataDir.resolve(".config.lock"), "rw").use { lockHandle ->
      val fileLock = lockHandle.channel.lock()
      try {
        val root = JSONObject(configFile.readText(Charsets.UTF_8))
        val credential = root.getJSONObject("credential")
        val packed = Base64.decode(credential.getString("ciphertext"), Base64.NO_WRAP)
        assertTrue(
          "test credential ciphertext must contain an authentication tag",
          packed.isNotEmpty()
        )
        packed[packed.lastIndex] = (packed.last().toInt() xor 0x01).toByte()
        credential.put("ciphertext", Base64.encodeToString(packed, Base64.NO_WRAP))
        AtomicConfigFileWriter().write(
          configFile,
          root.toString(2).toByteArray(Charsets.UTF_8)
        )
      } finally {
        fileLock.release()
      }
    }
  }

  private fun testConfig(
    server: LoopbackSseServer,
    topic: String,
    autoStart: Boolean
  ): PublicConfig {
    return PublicConfig(
      server = server.baseUrl,
      username = "",
      password = "",
      topic = topic,
      themeMode = "system",
      autoStart = autoStart,
      autoCopyOtp = false,
      allowInsecureHttp = false
    )
  }

  private fun randomTopic(): String {
    return "ci_${UUID.randomUUID().toString().replace("-", "").take(16)}"
  }

  private fun randomMessageId(): String {
    return UUID.randomUUID().toString().replace("-", "").take(12)
  }

  private data class ForegroundStatus(
    val title: String?,
    val text: String?,
    val subText: String?
  )
}
