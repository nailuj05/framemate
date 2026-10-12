package dev.framemate.app

import android.app.Notification
import android.content.Context
import android.content.pm.ApplicationInfo
import android.graphics.Bitmap
import android.service.notification.NotificationListenerService
import android.os.SystemClock
import android.service.notification.StatusBarNotification
import android.util.Base64
import androidx.core.graphics.drawable.toBitmap
import androidx.core.os.BundleCompat
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit

private const val ICON_PX = 96
/** Older than this when its turn comes (Frame asleep or away, sends timing out): dropped. */
private const val MAX_AGE_MS = 30_000L
/** Waiting sends; beyond that the oldest is dropped. */
private const val MAX_QUEUED = 10
/** Notification flags that mean "status, not news": never shown in the headset. */
private const val SKIP_FLAGS =
  Notification.FLAG_ONGOING_EVENT or Notification.FLAG_FOREGROUND_SERVICE or Notification.FLAG_GROUP_SUMMARY

/**
 * Forwards notifications of the apps the user picked to the Frame as toasts.
 *
 * Granted by the user under "Notification access" (sideloaded apps first need "Allow restricted
 * settings" in the app info menu since Android 13). Content goes to the Frame only, over the
 * pinned TLS connection, and is never logged or stored.
 */
class FrameNotificationListener : NotificationListenerService() {
  /**
   * One at a time and in order, off the main thread: sending blocks on the network (up to ~15 s
   * per notification while the Frame is unreachable), so the queue is short and drops the oldest.
   */
  private val sender = ThreadPoolExecutor(
    1, 1, 0, TimeUnit.MILLISECONDS, LinkedBlockingQueue(MAX_QUEUED), ThreadPoolExecutor.DiscardOldestPolicy(),
  )
  /**
   * Last content per notification key: apps re-post the same notification for progress etc. A new
   * message with the same text (e.g. a second "ok") has a new `when`, so it still goes through.
   */
  private val lastText = LinkedHashMap<String, Int>()
  private val icons = HashMap<String, String?>()

  override fun onNotificationPosted(sbn: StatusBarNotification) {
    if (sbn.packageName == packageName) return
    val notification = sbn.notification ?: return
    if (notification.flags and SKIP_FLAGS != 0 || !sbn.isClearable) return

    val prefs = NotificationPrefs(this)
    val app = appInfo(sbn)
    val label = app?.loadLabel(packageManager)?.toString() ?: sbn.packageName
    prefs.rememberApp(sbn.packageName, label)
    if (!prefs.enabled || !prefs.isAppEnabled(sbn.packageName)) return

    val extras = notification.extras
    val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty()
    val body = (extras.getCharSequence(Notification.EXTRA_BIG_TEXT) ?: extras.getCharSequence(Notification.EXTRA_TEXT))
      ?.toString().orEmpty()
    if (title.isBlank() && body.isBlank()) return
    // ONLY_ALERT_ONCE: the app itself says re-posts aren't news, even if it bumps `when`.
    val time = if (notification.flags and Notification.FLAG_ONLY_ALERT_ONCE != 0) 0L else notification.`when`
    val fingerprint = (title + "\u0000" + body + "\u0000" + time).hashCode()
    synchronized(lastText) {
      if (lastText[sbn.key] == fingerprint) return
      lastText[sbn.key] = fingerprint
      while (lastText.size > 200) lastText.remove(lastText.keys.first())
    }

    val json = JSONObject()
      .put("app", label)
      .put("title", title)
      .put("body", body)
      .put("icon", icon(sbn.packageName, app) ?: JSONObject.NULL)
      .toString()
    val posted = SystemClock.elapsedRealtime()
    sender.execute {
      // A toast for something that happened minutes ago would only confuse.
      if (SystemClock.elapsedRealtime() - posted > MAX_AGE_MS) return@execute
      prefs.recordResult(NotificationBridge.send(dataDir.absolutePath, json))
    }
  }

  override fun onDestroy() {
    sender.shutdown()
    super.onDestroy()
  }

  /**
   * The posting app, from the notification itself (`android.appInfo`, what the system uses for
   * the notification header). `getApplicationInfo(pkg)` fails for most apps since Android 11's
   * package visibility rules, which silently dropped their notifications.
   */
  private fun appInfo(sbn: StatusBarNotification): ApplicationInfo? =
    BundleCompat.getParcelable(sbn.notification.extras, "android.appInfo", ApplicationInfo::class.java)
      ?: runCatching { packageManager.getApplicationInfo(sbn.packageName, 0) }.getOrNull()

  /** The app's launcher icon as a small PNG data URI; the agent accepts up to 64 KB. */
  private fun icon(pkg: String, app: ApplicationInfo?): String? = icons.getOrPut(pkg) {
    runCatching {
      val bitmap = app!!.loadIcon(packageManager).toBitmap(ICON_PX, ICON_PX, Bitmap.Config.ARGB_8888)
      val png = ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
      "data:image/png;base64," + Base64.encodeToString(png, Base64.NO_WRAP)
    }.getOrNull()
  }

  companion object {
    /** Sends a test toast the same way a real notification would go. Blocking. */
    fun sendTest(context: Context): String {
      // The bell is the master switch: off means nothing reaches the headset, tests included.
      if (!NotificationPrefs(context).enabled) return "disabled".also { NotificationPrefs(context).recordResult(it) }
      val json = JSONObject().put("app", "FrameMate").put("title", "FrameMate")
        .put("body", "Notifications from your phone will show up like this.").toString()
      return NotificationBridge.send(context.dataDir.absolutePath, json).also { NotificationPrefs(context).recordResult(it) }
    }
  }
}
