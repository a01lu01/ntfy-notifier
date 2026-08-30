package app.ntfy.notifier

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class NativeSubscriberTest {
  @Test
  fun sessionsRejectCallbacksFromEveryPreviousGeneration() {
    val sessions = SubscriberSessionTracker()

    assertFalse(sessions.hasIssued(0))
    val first = sessions.advance()
    assertTrue(sessions.isCurrent(first))
    val second = sessions.advance()

    assertFalse(sessions.isCurrent(first))
    assertTrue(sessions.isCurrent(second))
    assertTrue(sessions.hasIssued(first))
    assertTrue(sessions.hasIssued(second))
    assertFalse(sessions.hasIssued(second + 1))
    assertNotEquals(first, second)
  }

  @Test
  fun newerOwnerProtectsItsSubscriptionFromEveryOlderReleaseAndCleanup() {
    val owners = SubscriberOwnerLeaseRegistry()
    val oldSink = RecordingSink()
    val newSink = RecordingSink()
    val oldLease = owners.claim(oldSink)
    val newLease = owners.claim(newSink)

    assertFalse(owners.isOwner(oldLease))
    assertTrue(owners.isOwner(newLease))
    assertFalse(owners.release(oldLease))
    assertSame(newSink, owners.currentSink())
    assertFalse(owners.isUnowned())

    assertTrue(owners.release(newLease))
    assertTrue(owners.isUnowned())
  }

  @Test
  fun processCallbackRoutesCommittedMessagesToTheNewestOwner() {
    val oldSink = RecordingSink()
    val newSink = RecordingSink()
    val oldLease = SubscriberProcessControl.claim(oldSink)
    val newLease = SubscriberProcessControl.claim(newSink)
    val committedSession = SubscriberProcessSession.tracker.advance()

    try {
      SubscriberProcessControl.callback.onNativeMessage(
        committedSession,
        "title",
        "body",
        null
      )

      assertEquals(0, oldSink.messages)
      assertEquals(1, newSink.messages)
      assertFalse(SubscriberProcessControl.release(oldLease))
      assertTrue(SubscriberProcessControl.isOwner(newLease))
    } finally {
      SubscriberProcessControl.release(newLease)
    }
  }

  @Test
  fun processSessionSequenceSurvivesServiceOwnerReplacement() {
    val firstServiceOwner = SubscriberProcessSession.tracker
    val firstSession = firstServiceOwner.advance()
    val recreatedServiceOwner = SubscriberProcessSession.tracker
    val recreatedSession = recreatedServiceOwner.advance()

    assertSame(firstServiceOwner, recreatedServiceOwner)
    assertTrue(recreatedSession > firstSession)
    assertFalse(recreatedServiceOwner.isCurrent(firstSession))
    assertTrue(recreatedServiceOwner.isCurrent(recreatedSession))
  }

  @Test
  fun processControlQueueSurvivesServiceOwnerReplacement() {
    val firstServiceOwner = SubscriberProcessControl.executor
    val recreatedServiceOwner = SubscriberProcessControl.executor

    assertSame(firstServiceOwner, recreatedServiceOwner)
  }

  @Test
  fun subscriberPreferencesDoNotChangeConnectionIdentity() {
    val original = subscriberConfig(autoStart = false, autoCopyOtp = false)
    val changedPreferences = subscriberConfig(autoStart = true, autoCopyOtp = true)

    assertEquals(
      original.toNativeSubscriptionConfig(),
      changedPreferences.toNativeSubscriptionConfig()
    )
  }

  @Test
  fun connectionIdentityRedactsAllFieldsFromDebugOutput() {
    val config = subscriberConfig(autoStart = false, autoCopyOtp = false)
      .toNativeSubscriptionConfig()

    assertEquals("NativeSubscriptionConfig(<redacted>)", config.toString())
    assertFalse(config.toString().contains("secret"))
    assertFalse(config.toString().contains("ntfy.example.com"))
  }

  private fun subscriberConfig(autoStart: Boolean, autoCopyOtp: Boolean) = SubscriberConfig(
    server = "https://ntfy.example.com",
    username = "alice",
    password = "secret",
    topic = "alerts",
    autoStart = autoStart,
    autoCopyOtp = autoCopyOtp,
    allowInsecureHttp = false
  )

  private class RecordingSink : SubscriberEventSink {
    var messages = 0

    override fun onSubscriberState(session: Long, state: String) = Unit

    override fun onSubscriberMessage(
      session: Long,
      title: String,
      message: String,
      otp: String?
    ) {
      messages += 1
    }
  }
}
