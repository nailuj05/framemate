package dev.framemate.app

import android.content.ComponentName
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

// Android 17 (target SDK 37) blocks LAN traffic, WebView included, until this runtime
// permission ("Nearby devices") is granted. Declared in AndroidManifest.xml.
private const val LOCAL_NETWORK = "android.permission.ACCESS_LOCAL_NETWORK"
private const val ANDROID_17 = 37

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // The WebView's reconnect loop picks the connection up once it's granted.
    if (!localNetworkAllowed()) requestPermissions(arrayOf(LOCAL_NETWORK), 1)
  }

  private fun localNetworkAllowed() =
    Build.VERSION.SDK_INT < ANDROID_17 || checkSelfPermission(LOCAL_NETWORK) == PackageManager.PERMISSION_GRANTED

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    webView.addJavascriptInterface(NativeBridge(), "FrameMateAndroid")
  }

  /** `window.FrameMateAndroid`: things the WebView can't do itself. */
  inner class NativeBridge {
    /** Landscape without system bars for the headset view; WebViews can't lock orientation. */
    @JavascriptInterface
    fun setFullscreen(enabled: Boolean) = runOnUiThread {
      requestedOrientation =
        if (enabled) ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
        else ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
      val bars = WindowCompat.getInsetsController(window, window.decorView)
      if (enabled) {
        bars.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        bars.hide(WindowInsetsCompat.Type.systemBars())
      } else {
        bars.show(WindowInsetsCompat.Type.systemBars())
      }
    }

    /** System browser instead of navigating the WebView away. */
    @JavascriptInterface
    fun openUrl(url: String) = runOnUiThread {
      val uri = Uri.parse(url)
      if (uri.scheme == "https" || uri.scheme == "http") {
        startActivity(Intent(Intent.ACTION_VIEW, uri))
      }
    }

    @JavascriptInterface
    fun localNetworkAllowed() = this@MainActivity.localNetworkAllowed()

    /** Whether the user granted "Notification access" to FrameNotificationListener. */
    @JavascriptInterface
    fun notificationAccessGranted(): Boolean {
      val enabled = Settings.Secure.getString(contentResolver, "enabled_notification_listeners").orEmpty()
      val ours = ComponentName(this@MainActivity, FrameNotificationListener::class.java)
      return enabled.split(':').any { ComponentName.unflattenFromString(it) == ours }
    }

    @JavascriptInterface
    fun openNotificationAccessSettings() = runOnUiThread {
      startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS))
    }

    /** `{enabled, apps: [{pkg, label, enabled}], last: {result, time} | null}` */
    @JavascriptInterface
    fun notificationSettings(): String = NotificationPrefs(this@MainActivity).toJson()

    @JavascriptInterface
    fun setNotificationsEnabled(enabled: Boolean) {
      NotificationPrefs(this@MainActivity).enabled = enabled
    }

    @JavascriptInterface
    fun setNotificationAppEnabled(pkg: String, enabled: Boolean) =
      NotificationPrefs(this@MainActivity).setAppEnabled(pkg, enabled)

    /** Returns at once; the result lands in `notificationSettings().last`. */
    @JavascriptInterface
    fun sendTestNotification() {
      Thread { FrameNotificationListener.sendTest(this@MainActivity) }.start()
    }

    /** After two denials Android stops prompting; only the app's settings page is left. */
    @JavascriptInterface
    fun openAppSettings() = runOnUiThread {
      startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", packageName, null)))
    }
  }
}
