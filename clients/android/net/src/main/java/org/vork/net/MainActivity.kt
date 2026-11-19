package org.vork.net

import android.Manifest
import android.annotation.SuppressLint
import android.app.AlertDialog
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.webkit.JavascriptInterface
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import org.json.JSONObject
import java.util.concurrent.Executors

/**
 * vØrk on Android, the network alone: the node's web dashboard
 * (src/web/index.html, the same page as on the desktop and a server), in a
 * web view. Its requests go to this app's node over the control socket,
 * through [Bridge], not over the network; the bridge also does what a page
 * can't: the VPN (Android's consent), the camera (scanning a pairing code),
 * the clipboard, restarting the node.
 */
class MainActivity : ComponentActivity() {
    private lateinit var web: WebView
    private val node by lazy { Node(this) }
    /** The node's answers come off the UI thread (it may take up to 5 s not to answer). */
    private val calls = Executors.newFixedThreadPool(3)

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        NodeService.start(this)
        WebView.setWebContentsDebuggingEnabled(applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0)
        web = WebView(this).apply {
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.allowFileAccess = false
            settings.allowContentAccess = false
            // confirm() and alert() as dialogs.
            webChromeClient = WebChromeClient()
            webViewClient = object : WebViewClient() {
                // Only the dashboard stays in the window; a link elsewhere opens in the browser.
                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                    val url = request.url
                    if (url.toString().startsWith(HOME)) return false
                    runCatching { startActivity(Intent(Intent.ACTION_VIEW, url)) }
                    return true
                }
            }
            addJavascriptInterface(Bridge(), "VorkApp")
        }
        setContentView(web)
        val page = assets.open("index.html").bufferedReader().use { it.readText() }
        web.loadDataWithBaseURL(HOME, page, "text/html", "utf-8", null)
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                web.evaluateJavascript("window.vorkBack ? vorkBack() : false") { went -> if (went != "true") finish() }
            }
        })
        opened(intent)
    }

    override fun onResume() {
        super.onResume()
        web.onResume()
        web.resumeTimers()
        web.evaluateJavascript("window.vorkResume && vorkResume()", null)
    }

    override fun onPause() {
        // Not asking the node every 5 s while nobody looks.
        web.pauseTimers()
        web.onPause()
        super.onPause()
    }

    override fun onDestroy() {
        calls.shutdownNow()
        web.destroy()
        super.onDestroy()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        opened(intent)
    }

    /** What the page calls (window.VorkApp). Only this app's own page is ever loaded in the window. */
    inner class Bridge {
        /** A control request; the answer comes back as vorkAnswer(id, text, ok). */
        @JavascriptInterface
        fun ask(id: Int, request: String) {
            calls.execute {
                val answer = node.askLine(request)
                runOnUiThread {
                    if (!isDestroyed) web.evaluateJavascript("vorkAnswer($id, ${JSONObject.quote(answer ?: "")}, ${answer != null})", null)
                }
            }
        }

        @JavascriptInterface
        fun copy(text: String) {
            (getSystemService(CLIPBOARD_SERVICE) as ClipboardManager).setPrimaryClip(ClipData.newPlainText("vØrk", text))
        }

        @JavascriptInterface
        fun scan() = runOnUiThread { scanCode() }

        @JavascriptInterface
        fun vpn(on: Boolean) = runOnUiThread {
            if (on) turnOnVpn() else {
                VorkVpn.stop(this@MainActivity)
                said("Disconnected: apps no longer reach vØrk addresses")
            }
        }

        /** What the VPN is doing or what went wrong ("" when running or off). */
        @JavascriptInterface
        fun vpnNote(): String = VorkVpn.state ?: ""

        @JavascriptInterface
        fun restart() = NodeService.restart(this@MainActivity)

        @JavascriptInterface
        fun stop() = NodeService.stop(this@MainActivity)

        @JavascriptInterface
        fun version(): String = runCatching { packageManager.getPackageInfo(packageName, 0).versionName }.getOrNull() ?: ""
    }

    /** A message on the page (and the page refreshed). */
    private fun said(text: String, bad: Boolean = false) =
        web.evaluateJavascript("window.vorkSaid && vorkSaid(${JSONObject.quote(text)}, $bad)", null)

    /** A node call off the UI thread; its error, or what to say when it went through. */
    private fun call(ok: String, f: () -> JSONObject) = calls.execute {
        val r = f()
        runOnUiThread { r.optString("error").let { if (it.isEmpty()) said(ok) else said(it, true) } }
    }

    /** A vork-link: from another device (its QR code scanned with the camera app, or shared): asked before linking. */
    private fun opened(intent: Intent?) {
        val data = intent?.dataString ?: return
        if (!data.startsWith("vork-link:")) return
        val link = parseLink(data)
        if (link == null) {
            AlertDialog.Builder(this).setMessage("This link isn't a vØrk device link.").setPositiveButton("OK", null).show()
            return
        }
        AlertDialog.Builder(this)
            .setTitle("Link with ${link.name.ifEmpty { link.address.take(8) }}?")
            .setMessage("It becomes one of your devices, and this phone one of its: they reach each other's services and link directly.")
            .setPositiveButton("Link") { _, _ -> call("Linking…") { node.link(link.address, link.code, link.addrs) } }
            .setNegativeButton("Cancel", null)
            .show()
    }

    /** Scans another device's pairing code with the app's own scanner (ZXing, on the phone). */
    private val scanner = registerForActivityResult(com.journeyapps.barcodescanner.ScanContract()) { result ->
        val text = result.contents ?: return@registerForActivityResult // cancelled
        val link = parseLink(text)
        if (link == null || !text.startsWith("vork-link:")) {
            said("That isn't a vØrk pairing code", true)
            return@registerForActivityResult
        }
        // Scanned on purpose: linked at once, no second question.
        call("Linking with ${link.name.ifEmpty { link.address.take(8) }}…") { node.link(link.address, link.code, link.addrs) }
    }

    private fun scanCode() {
        val options = com.journeyapps.barcodescanner.ScanOptions()
            .setDesiredBarcodeFormats(com.journeyapps.barcodescanner.ScanOptions.QR_CODE)
            .setPrompt("Scan the other device's vØrk pairing code")
            .setBeepEnabled(false)
            .setOrientationLocked(false)
        scanner.launch(options)
    }

    /** The system's VPN consent first, if it hasn't been given; then the interface. */
    private fun turnOnVpn() {
        try {
            val ask = android.net.VpnService.prepare(this)
            if (ask != null) {
                asked = System.currentTimeMillis()
                @Suppress("DEPRECATION")
                startActivityForResult(ask, REQUEST_VPN)
            } else {
                vpnAllowed()
            }
        } catch (e: Exception) {
            said("Couldn't turn it on: ${e.message ?: e.javaClass.simpleName}", true)
        }
    }

    private var asked = 0L

    private fun vpnAllowed() {
        getSharedPreferences("vork", MODE_PRIVATE).edit().putBoolean("vpn", true).apply()
        VorkVpn.startIfAllowed(this)
        said("Turning on the interface for apps…")
    }

    @Deprecated("Activity result API")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQUEST_VPN) {
            when {
                resultCode == RESULT_OK -> vpnAllowed()
                // Refused before anyone could answer: another app holds an always-on VPN.
                System.currentTimeMillis() - asked < 1_500 -> said("Another VPN app is set to always-on, and Android allows one VPN at a time: turn always-on off for it (Settings → VPN), then try again", true)
                else -> said("The VPN wasn't allowed, so apps can't reach vØrk addresses", true)
            }
        }
    }

    companion object {
        const val REQUEST_VPN = 71
        /** Where the page lives (never fetched: it comes from the APK). */
        const val HOME = "https://vork.localhost/"
    }
}

/** A pairing link: the other node, its one-time code, its name, and where it can be dialed. */
data class PairLink(val address: String, val code: String, val name: String, val addrs: List<String>)

/** `vork-link:ADDRESS?c=CODE&n=NAME&a=HOST:PORT,…`, or a bare address. */
fun parseLink(text: String): PairLink? {
    val t = text.trim()
    val body = t.removePrefix("vork-link:").removePrefix("vork:link/")
    val address = body.substringBefore('?').lowercase()
    if (address.length != 32 || !address.all { it in "0123456789abcdef" }) return null
    val query = body.substringAfter('?', "").split('&').mapNotNull { kv -> kv.split('=', limit = 2).takeIf { it.size == 2 }?.let { it[0] to java.net.URLDecoder.decode(it[1], "UTF-8") } }.toMap()
    return PairLink(address, query["c"] ?: "", query["n"] ?: "", query["a"].orEmpty().split(',').filter { it.isNotBlank() })
}
