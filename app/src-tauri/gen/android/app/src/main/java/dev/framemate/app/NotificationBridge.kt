package dev.framemate.app

/** JNI into the app's Rust library (src-tauri/src/forward.rs). */
object NotificationBridge {
  init {
    // Already loaded when the app's window runs; the listener service can start without it.
    System.loadLibrary("framemate_app_lib")
  }

  /**
   * Sends one toast to the paired Frame; blocks for up to ~15 s, never call it on the main
   * thread. Returns `shown`, `headset_asleep`, `rate_limited`, `not_paired` or `error: …`.
   */
  @JvmStatic
  external fun send(dataDir: String, json: String): String
}
