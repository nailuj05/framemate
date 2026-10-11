package dev.framemate.app

import android.app.Notification
import android.content.Context
import android.content.pm.ApplicationInfo
import android.graphics.Bitmap
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Base64
import androidx.core.graphics.drawable.toBitmap
import androidx.core.os.BundleCompat
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.util.concurrent.Executors

private const val ICON_PX = 96
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
  /** One at a time and in order, off the main thread: sending blocks on the network. */
  private val sender = Executors.newSingleThreadExecutor()
  /** Last text per notification key: apps re-post the same notification for progress etc. */
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
    val fingerprint = (title + "\u0000" + body).hashCode()
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
    sender.execute { prefs.recordResult(NotificationBridge.send(dataDir.absolutePath, json)) }
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
