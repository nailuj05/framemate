# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}
# NativeBridge, called from JS as `window.FrameMateAndroid` (see MainActivity.kt).
-keepclassmembers class dev.framemate.app.MainActivity$NativeBridge {
   @android.webkit.JavascriptInterface <methods>;
}

# ML Kit (QR pairing via tauri-plugin-barcode-scanner) creates its component registrars by
# reflection from manifest meta-data. Its own rule keeps only their names, and R8 full mode
# then drops the constructor and getComponents(): no SharedPrefManager etc. get registered,
# and the first camera frame crashes with an NPE in mlkit_vision_common.
-keep class * implements com.google.firebase.components.ComponentRegistrar {
   <init>();
   *;
}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile