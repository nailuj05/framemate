package dev.framemate.app

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

/**
 * Forwarding settings, shared by the listener service and the WebView bridge. Kept in
 * SharedPreferences rather than the WebView's storage: the service runs without the WebView.
 * Nothing here stores notification content, only app names and the last send status.
 */
class NotificationPrefs(context: Context) {
  private val prefs = context.getSharedPreferences("notifications", Context.MODE_PRIVATE)

  var enabled: Boolean
    get() = prefs.getBoolean("enabled", false)
    set(value) = prefs.edit().putBoolean("enabled", value).apply()

  /** Every app is forwarded unless switched off here; the bell (`enabled`) is the opt-in. */
  fun isAppEnabled(pkg: String) = pkg !in disabledApps()

  fun setAppEnabled(pkg: String, value: Boolean) {
    val disabled = disabledApps().toMutableSet()
    if (value) disabled.remove(pkg) else disabled.add(pkg)
    prefs.edit().putStringSet("disabled_apps", disabled).apply()
  }

  private fun disabledApps(): Set<String> = prefs.getStringSet("disabled_apps", emptySet()) ?: emptySet()

  /** Apps that posted a notification since forwarding was set up, so the user can pick them. */
  fun rememberApp(pkg: String, label: String) {
    val seen = seenApps()
    if (seen.optString(pkg) == label) return
    seen.put(pkg, label)
    prefs.edit().putString("seen", seen.toString()).apply()
  }

  private fun seenApps() = runCatching { JSONObject(prefs.getString("seen", "{}")!!) }.getOrElse { JSONObject() }

  fun recordResult(result: String) {
    prefs.edit().putString("last_result", result).putLong("last_time", System.currentTimeMillis()).apply()
  }

  /** For the settings page: `{enabled, apps: [{pkg, label, enabled}], last: {result, time}}`. */
  fun toJson(): String {
    val seen = seenApps()
    val apps = JSONArray()
    seen.keys().asSequence()
      .sortedBy { seen.optString(it).lowercase() }
      .forEach { pkg -> apps.put(JSONObject().put("pkg", pkg).put("label", seen.optString(pkg)).put("enabled", isAppEnabled(pkg))) }
    val last = prefs.getString("last_result", null)?.let {
      JSONObject().put("result", it).put("time", prefs.getLong("last_time", 0))
    }
    return JSONObject().put("enabled", enabled).put("apps", apps).put("last", last ?: JSONObject.NULL).toString()
  }
}
