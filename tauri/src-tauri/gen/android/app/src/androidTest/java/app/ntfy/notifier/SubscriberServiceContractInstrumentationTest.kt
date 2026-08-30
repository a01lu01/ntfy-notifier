package app.ntfy.notifier

import android.content.ComponentName
import android.content.Context
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.lang.reflect.Modifier

@RunWith(AndroidJUnit4::class)
class SubscriberServiceContractInstrumentationTest {
  private val context: Context = ApplicationProvider.getApplicationContext()

  @Suppress("DEPRECATION")
  @Test
  fun serviceRunsInDedicatedPersistentSubscriberProcess() {
    val info = context.packageManager.getServiceInfo(
      ComponentName(context, NotificationService::class.java),
      PackageManager.GET_META_DATA
    )

    assertEquals("${context.packageName}:subscriber", info.processName)
    assertEquals(0, info.flags and ServiceInfo.FLAG_STOP_WITH_TASK)
  }

  @Test
  fun controlActionsAreExplicitAndCarryNoConfiguration() {
    listOf(
      NotificationService.ACTION_START,
      NotificationService.ACTION_RECONFIGURE,
      NotificationService.ACTION_STOP
    ).forEach { action ->
      val intent = NotificationService.actionIntent(context, action)
      assertEquals(action, intent.action)
      assertEquals(NotificationService::class.java.name, intent.component?.className)
      assertTrue(intent.extras == null || intent.extras!!.isEmpty)
    }
  }

  @Test
  fun nativeEntryPointsKeepTheExactJniContract() {
    val nativeClass = Class.forName(
      "app.ntfy.notifier.NativeSubscriber",
      false,
      javaClass.classLoader
    )
    val callbackClass = Class.forName(
      "app.ntfy.notifier.SubscriberCallback",
      false,
      javaClass.classLoader
    )
    val parameterTypes = arrayOf(
      String::class.java,
      java.lang.Long.TYPE,
      String::class.java,
      String::class.java,
      String::class.java,
      String::class.java,
      java.lang.Boolean.TYPE,
      callbackClass
    )

    listOf("nativeStart", "nativeReconfigure").forEach { name ->
      val method = nativeClass.getDeclaredMethod(name, *parameterTypes)
      assertTrue(Modifier.isStatic(method.modifiers))
      assertTrue(Modifier.isNative(method.modifiers))
      assertEquals(java.lang.Boolean.TYPE, method.returnType)
    }
    val stop = nativeClass.getDeclaredMethod("nativeStop")
    assertTrue(Modifier.isStatic(stop.modifiers))
    assertTrue(Modifier.isNative(stop.modifiers))
    assertEquals(java.lang.Void.TYPE, stop.returnType)

    assertNotNull(
      callbackClass.getDeclaredMethod(
        "onNativeState",
        java.lang.Long.TYPE,
        String::class.java
      )
    )
    assertNotNull(
      callbackClass.getDeclaredMethod(
        "onNativeMessage",
        java.lang.Long.TYPE,
        String::class.java,
        String::class.java,
        String::class.java
      )
    )
  }
}
