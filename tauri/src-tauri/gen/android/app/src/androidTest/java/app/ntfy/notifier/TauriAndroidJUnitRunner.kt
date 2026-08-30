package app.ntfy.notifier

import android.os.Bundle
import androidx.test.runner.AndroidJUnitRunner

/**
 * Keeps Tauri activities alive until instrumentation has reported its result.
 *
 * AndroidJUnitRunner normally finishes every activity after each test. Destroying Tauri's final
 * Wry activity intentionally terminates the shared host process before the runner can report, so
 * the isolated lifecycle test backgrounds the singleTask activity and lets Gradle's package
 * cleanup remove it. Every other instrumentation run retains AndroidJUnitRunner's normal cleanup.
 */
class TauriAndroidJUnitRunner : AndroidJUnitRunner() {
  private var preserveTauriActivity = false

  override fun onCreate(arguments: Bundle) {
    preserveTauriActivity = arguments.getString(ARG_PRESERVE_TAURI_ACTIVITY)
      .equals("true", ignoreCase = true)
    if (preserveTauriActivity) {
      check(arguments.getString("class") == ISOLATED_LIFECYCLE_TEST) {
        "$ARG_PRESERVE_TAURI_ACTIVITY may only run $ISOLATED_LIFECYCLE_TEST"
      }
    }
    super.onCreate(arguments)
  }

  override fun shouldWaitForActivitiesToComplete(): Boolean {
    return !preserveTauriActivity && super.shouldWaitForActivitiesToComplete()
  }

  companion object {
    const val ARG_PRESERVE_TAURI_ACTIVITY = "preserveTauriActivity"
    const val ISOLATED_LIFECYCLE_TEST =
      "app.ntfy.notifier.SubscriberBootRecoveryInstrumentationTest#" +
        "enabledBootPersistsAndRepeatedBootReconfigureAndActivityDoNotDuplicateSse"
  }
}
