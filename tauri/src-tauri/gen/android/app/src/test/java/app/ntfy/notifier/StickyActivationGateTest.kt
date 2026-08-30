package app.ntfy.notifier

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class StickyActivationGateTest {
  @Test
  fun activationIsOneShotAndRequiresTheArmedRequest() {
    val gate = StickyActivationGate()

    assertFalse(gate.arm(0))
    assertTrue(gate.arm(7))
    assertFalse(gate.consume(8) { true })
    assertTrue(gate.consume(7) { true })
    assertFalse(gate.consume(7) { true })
  }

  @Test
  fun stopInvalidatesEveryDelayedActivation() {
    val gate = StickyActivationGate()

    assertTrue(gate.arm(11))
    gate.clear()

    assertFalse(gate.consume(11) { true })
  }

  @Test
  fun failedLifecycleValidationConsumesTheActivation() {
    val gate = StickyActivationGate()

    assertTrue(gate.arm(13))
    assertFalse(gate.consume(13) { false })
    assertFalse(gate.consume(13) { true })
  }

  @Test
  fun delayedActivationAfterStopCannotRestoreStickyLifecycle() {
    val gate = StickyActivationGate()
    assertTrue(gate.arm(17))
    gate.clear()

    val decision = gate.decide(
      request = 17,
      activationEligible = { false },
      newerSubscriptionActive = { false }
    )

    assertTrue(decision == StickyActivationDecision.STOP_NOT_STICKY)
  }

  @Test
  fun delayedActivationCannotStopANewerStickySubscription() {
    val gate = StickyActivationGate()
    assertTrue(gate.arm(19))
    gate.clear()

    val decision = gate.decide(
      request = 19,
      activationEligible = { false },
      newerSubscriptionActive = { true }
    )

    assertTrue(decision == StickyActivationDecision.KEEP_CURRENT_STICKY)
  }

  @Test
  fun newerStickyRequestMakesAnOlderBootRequestNonAuthoritative() {
    val gate = BootRequestGate()

    assertFalse(gate.hasStickyRequest())
    gate.markStickyRequest()

    assertTrue(gate.hasStickyRequest())
  }
}
