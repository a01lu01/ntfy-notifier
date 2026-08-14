package app.ntfy.notifier

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import com.why.ntfy_notifier.MainActivity

class BootReceiver : BroadcastReceiver() {
  override fun onReceive(context: Context, intent: Intent) {
    if (intent.action == Intent.ACTION_BOOT_COMPLETED && NotificationService.autoStart(context)) {
      // 启动主 Activity 走完整初始化链（Rust SSE 订阅 + 前台服务），
      // 只启动 Service 不会拉起 Rust 订阅线程，重启后收不到消息。
      val launch = Intent(context, MainActivity::class.java).apply {
        addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
      }
      context.startActivity(launch)
    }
  }
}
