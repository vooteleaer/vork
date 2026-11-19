# The page calls these by name (window.VorkApp in src/web/index.html).
-keepclassmembers class org.vork.net.MainActivity$Bridge {
    @android.webkit.JavascriptInterface <methods>;
}
-keepattributes JavascriptInterface
