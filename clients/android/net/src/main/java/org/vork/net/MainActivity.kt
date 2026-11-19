package org.vork.net

import android.Manifest
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/**
 * vØrk on Android, the network alone (storage is an app of its own on top):
 * Overview (this device, on/off), My devices (paired by a QR code shown on
 * one and scanned with the other), Settings. Like Tailscale, without a server.
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        NodeService.start(this)
        opened(intent)
        setContent {
            MaterialTheme(colorScheme = if (isSystemInDarkTheme()) darkColorScheme() else lightColorScheme()) {
                App(Node(this))
            }
        }
    }

    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent)
        opened(intent)
    }

    /** A vork-link: from another device: the app asks before linking. */
    private fun opened(intent: android.content.Intent?) {
        val data = intent?.dataString ?: return
        if (data.startsWith("vork-link:")) LinkOpened.uri = data
    }

    /** Scans another device's pairing code with the app's own scanner (ZXing, on the phone). */
    private val scanner = registerForActivityResult(com.journeyapps.barcodescanner.ScanContract()) { result ->
        val text = result.contents ?: return@registerForActivityResult // cancelled
        paired(text)
    }

    fun scanCode() {
        val options = com.journeyapps.barcodescanner.ScanOptions()
            .setDesiredBarcodeFormats(com.journeyapps.barcodescanner.ScanOptions.QR_CODE)
            .setPrompt("Scan the other device's vØrk pairing code")
            .setBeepEnabled(false)
            .setOrientationLocked(false)
        scanner.launch(options)
    }

    private fun paired(text: String) {
        val link = parseLink(text)
        if (link == null || !text.startsWith("vork-link:")) {
            toast("That isn't a vØrk pairing code")
            return
        }
        // Scanned on purpose: paired at once, no second question.
        val (address, c, name) = link
        val node = Node(this)
        Thread {
            val r = node.link(address, c, link.addrs)
            runOnUiThread { toast(r.optString("error").ifEmpty { "Pairing with ${name.ifEmpty { address.take(8) }}…" }) }
        }.start()
    }

    /** The system's VPN consent first, if it hasn't been given; then the interface. */
    fun turnOnVpn() {
        try {
            val ask = android.net.VpnService.prepare(this)
            if (ask != null) {
                toast("Android asks whether vØrk may set up a VPN (fd00::/8 only)")
                asked = System.currentTimeMillis()
                @Suppress("DEPRECATION")
                startActivityForResult(ask, REQUEST_VPN)
            } else {
                vpnAllowed()
            }
        } catch (e: Exception) {
            toast("Couldn't turn it on: ${e.message ?: e.javaClass.simpleName}")
        }
    }

    private var asked = 0L

    private fun vpnAllowed() {
        getSharedPreferences("vork", MODE_PRIVATE).edit().putBoolean("vpn", true).apply()
        VorkVpn.startIfAllowed(this)
        toast("Turning on the interface for apps…")
    }

    fun toast(text: String) = android.widget.Toast.makeText(this, text, android.widget.Toast.LENGTH_LONG).show()

    @Deprecated("Activity result API")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: android.content.Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQUEST_VPN) {
            when {
                resultCode == RESULT_OK -> vpnAllowed()
                // Refused before anyone could answer: another app holds an always-on VPN.
                System.currentTimeMillis() - asked < 1_500 -> toast("Another VPN app is set to always-on, and Android allows one VPN at a time: turn always-on off for it (Settings → VPN), then Turn on again")
                else -> toast("The VPN wasn't allowed, so apps can't reach vØrk addresses")
            }
        }
    }

    companion object {
        const val REQUEST_VPN = 71
    }
}

/** Runs a node call off the UI thread. */
suspend fun <T> io(f: () -> T): T = withContext(Dispatchers.IO) { f() }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun App(node: Node) {
    var tab by remember { mutableIntStateOf(0) }
    var status by remember { mutableStateOf(JSONObject()) }
    var tick by remember { mutableIntStateOf(0) }
    var note by remember { mutableStateOf("") }
    LaunchedEffect(tick) {
        while (true) {
            status = io { node.status() }
            delay(if (status.has("error")) 1_000 else 3_000)
        }
    }
    val said: (String) -> Unit = { note = it; tick++ }
    val scope = rememberCoroutineScope()
    // A link opened from another device: link with it?
    LinkOpened.uri?.let { uri ->
        val parsed = parseLink(uri)
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { LinkOpened.uri = null },
            title = { Text("Link with ${parsed?.name?.ifEmpty { null } ?: parsed?.address?.take(8) ?: "?"}?") },
            text = { Text(if (parsed == null) "This link isn't a vØrk device link." else "It becomes one of your devices, and this phone one of its: they see each other in My devices and link directly.") },
            confirmButton = {
                TextButton(enabled = parsed != null, onClick = {
                    LinkOpened.uri = null
                    val p = parsed!!
                    scope.launch { said(io { node.link(p.address, p.code, p.addrs) }.optString("error").ifEmpty { "Linking…" }) }
                }) { Text("Link") }
            },
            dismissButton = { TextButton(onClick = { LinkOpened.uri = null }) { Text("Cancel") } },
        )
    }
    // Another device asks to be one of this owner's devices: Allow or Dismiss.
    var later by remember { mutableStateOf(setOf<String>()) }
    val asking = (status.optJSONObject("links")?.optJSONArray("requests") ?: JSONArray()).let { a -> (0 until a.length()).map { a.getJSONObject(it) } }
        .firstOrNull { it.optString("address") !in later }
    if (asking != null) {
        val address = asking.optString("address")
        val who = asking.optString("name").ifEmpty { address.take(8) }
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { later = later + address },
            title = { Text("$who wants to be one of your devices") },
            text = {
                Text(
                    (if (asking.optBoolean("with_link")) "It came with a link you made. " else "") +
                        "Allowed, the two see each other in My devices and link directly. Address $address",
                )
            },
            confirmButton = { TextButton(onClick = { scope.launch { said(io { node.allow(address) }.optString("error").ifEmpty { "$who is one of your devices" }) } }) { Text("Allow") } },
            dismissButton = { TextButton(onClick = { scope.launch { said(io { node.dismiss(address) }.optString("error").ifEmpty { "Dismissed" }) } }) { Text("Dismiss") } },
        )
    }
    Scaffold(
        topBar = { TopAppBar(title = { Text(listOf("vØrk", "My devices", "Settings")[tab]) }) },
        bottomBar = {
            NavigationBar {
                NavigationBarItem(selected = tab == 0, onClick = { tab = 0 }, icon = { Icon(Icons.Filled.Home, null) }, label = { Text("Overview") })
                NavigationBarItem(selected = tab == 1, onClick = { tab = 1 }, icon = { Icon(Icons.Filled.Person, null) }, label = { Text("My devices") })
                NavigationBarItem(selected = tab == 2, onClick = { tab = 2 }, icon = { Icon(Icons.Filled.Settings, null) }, label = { Text("Settings") })
            }
        },
    ) { inner ->
        Column(Modifier.padding(inner).fillMaxSize()) {
            if (note.isNotEmpty()) {
                Text(note, Modifier.padding(horizontal = 16.dp, vertical = 4.dp), style = MaterialTheme.typography.bodySmall)
            }
            when (tab) {
                0 -> Overview(node, status, said)
                1 -> MyDevices(node, status, said)
                else -> SettingsTab(node, status, said)
            }
        }
    }
}

// ---- pieces ---------------------------------------------------------------------------

fun nodes(s: JSONObject): List<JSONObject> = (s.optJSONArray("nodes") ?: JSONArray()).let { a -> (0 until a.length()).map { a.getJSONObject(it) } }
fun strings(a: JSONArray?): List<String> = a?.let { (0 until it.length()).map { i -> it.getString(i) } } ?: emptyList()
fun displayName(n: JSONObject): String = n.optString("name").takeIf { !n.isNull("name") && it.isNotEmpty() } ?: n.optString("address").take(8)

/** The DNS name a node will have (vØrk ADR-0018): its name, made a label, under `.vork`. */
fun dnsName(name: String, address: String): String {
    val label = name.lowercase().replace(Regex("[^a-z0-9-]+"), "-").trim('-').take(40)
    return (label.ifEmpty { address.take(8) }) + ".vork"
}

/** How a node is reached, in a few words. */
fun reach(n: JSONObject): String = when {
    n.optBoolean("direct") -> "direct"
    else -> "${n.optInt("hops")} hop(s), relayed"
} + if (n.optBoolean("on_lan")) " · this network" else ""

@Composable
fun copier(said: (String) -> Unit): (String, String) -> Unit {
    val clipboard = LocalClipboardManager.current
    return { label, text ->
        clipboard.setText(AnnotatedString(text))
        said("$label copied: $text")
    }
}

/**
 * A device's IPv4 address, the same on every machine (ADR-0021): 198.18.x.y by
 * the first two bytes of its address (198.18.0.x left out), as the node derives it.
 */
fun derivedIpv4(address: String): String {
    if (address.length < 4) return ""
    var v = address.substring(0, 4).toInt(16)
    if (v < 256) v += 256
    return "198.18.${v shr 8}.${v and 255}"
}

/** A string field, empty when missing or null. */
fun JSONObject.text(key: String): String = if (isNull(key)) "" else optString(key)

/** A node's IPv4 and IPv6 addresses, DNS name and vØrk address, each with Copy. */
@Composable
fun AddressLines(ip: String, address: String, name: String, copy: (String, String) -> Unit, ipv4: String = "") {
    val mono = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace)
    for ((label, value) in listOf("IPv4" to ipv4, "IPv6" to ip, "DNS" to dnsName(name, address), "Address" to address)) {
        if (value.isEmpty()) continue
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(label, style = MaterialTheme.typography.labelSmall, modifier = Modifier.width(56.dp))
            Text(value, style = mono, modifier = Modifier.weight(1f))
            TextButton(onClick = { copy(label, value) }) { Text("Copy") }
        }
    }
}

/** One node: name, how it's reached, its addresses, and an action. */
@Composable
fun NodeCard(n: JSONObject, copy: (String, String) -> Unit, action: (@Composable () -> Unit)? = null) {
    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(displayName(n), style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                Text(reach(n), style = MaterialTheme.typography.labelSmall)
            }
            AddressLines(n.optString("ip"), n.optString("address"), displayName(n), copy, n.text("ipv4"))
            action?.invoke()
        }
    }
}

// ---- Overview ---------------------------------------------------------------------------

@Composable
fun Overview(node: Node, s: JSONObject, said: (String) -> Unit) {
    val context = LocalContext.current
    val copy = copier(said)
    val small = MaterialTheme.typography.bodySmall
    LazyColumn(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        // On or off, as in Tailscale: on, apps reach vØrk addresses (only
        // fd00::/8 goes through vØrk); off, everything bypasses it. The node
        // stays in the network either way (Settings → Stop stops it).
        item {
            val on = s.optJSONObject("interface") != null
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(if (on) "Connected" else "Disconnected", style = MaterialTheme.typography.titleLarge)
                            Text(
                                if (on) "Apps reach vØrk nodes at their fd… addresses; all other traffic goes around vØrk."
                                else "All traffic goes around vØrk; apps don't reach vØrk addresses.",
                                style = small,
                            )
                        }
                        Switch(checked = on, onCheckedChange = { want ->
                            if (want) (context as? MainActivity)?.turnOnVpn()
                            else { VorkVpn.stop(context); said("Disconnected") }
                        })
                    }
                    if (!on) VorkVpn.state?.let { Text(it, style = small) }
                    val via = s.optJSONObject("settings")?.optString("exit_via").orEmpty()
                    // Automatic exit node: the fastest of your devices offering it (ADR-0026).
                    val auto = s.optJSONObject("settings")?.optBoolean("exit_auto") == true
                    val scope = rememberCoroutineScope()
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text("Exit node: automatic", style = MaterialTheme.typography.titleSmall)
                            Text("Internet through the fastest of your devices that offer it; directly when none answers.", style = small)
                        }
                        Switch(checked = auto, onCheckedChange = { want ->
                            scope.launch { said(io { node.set("exit_via", if (want) "auto" else "") }.optString("error").ifEmpty { if (want) "The exit node is chosen automatically" else "Internet goes out directly" }) }
                        })
                    }
                    if (on && auto && via.isEmpty()) Text("None of your exit nodes answers: internet goes out directly", style = small)
                    if (on && via.isNotEmpty()) {
                        val name = nodes(s).firstOrNull { it.optString("address") == via }?.let { displayName(it) } ?: via.take(8)
                        val up = s.optJSONObject("exit")?.optBoolean("up") == true
                        Text(
                            if (up) "Internet goes out through $name" + (s.optJSONObject("exit")?.optLong("rtt_ms", -1)?.takeIf { it >= 0 }?.let { ", $it ms" } ?: "") + " (exit node; IPv4 and names, IPv6 held back)"
                            else "Exit node $name doesn't answer: internet goes out directly until it does",
                            style = small,
                        )
                    }
                }
            }
        }
        item {
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    if (s.has("error")) {
                        Text("The vØrk node isn't running", style = MaterialTheme.typography.titleMedium)
                        Text("It starts by itself in a moment; Restart tries at once.", style = small)
                    } else {
                        val name = s.optString("name")
                        Text("$name (this device)", style = MaterialTheme.typography.titleMedium)
                        val up = (0 until (s.optJSONArray("neighbors")?.length() ?: 0)).count { s.getJSONArray("neighbors").getJSONObject(it).optBoolean("up") }
                        Text("Running ${s.optLong("uptime_s") / 60} min · $up connected · ${nodes(s).size} node(s) known", style = small)
                        AddressLines(s.optString("ip"), s.optString("address"), name, copy, s.text("ipv4"))
                    }
                    Row(Modifier.padding(top = 8.dp)) {
                        OutlinedButton(onClick = { NodeService.restart(context); said("Restarting the node") }) { Text("Restart") }
                    }
                }
            }
        }
        item {
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    Text("Reachability", style = MaterialTheme.typography.titleMedium)
                    val by = s.optJSONObject("reach")?.optJSONArray("reached_by")?.length() ?: 0
                    Text(
                        if (by > 0) "Others reach this phone directly ($by node(s) did)."
                        else "No node has reached this phone directly yet: it reaches out, and others relay for it.",
                        style = small,
                    )
                    val outside = strings(s.optJSONArray("outside"))
                    if (outside.isNotEmpty()) Text("Seen from outside at ${outside.joinToString()}", style = small)
                }
            }
        }
    }
}

// ---- My devices ---------------------------------------------------------------------------

/** A `vork-link:` opened from another device's QR code or a shared link: confirmed before linking. */
object LinkOpened {
    var uri by mutableStateOf<String?>(null)
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

@Composable
fun MyDevices(node: Node, s: JSONObject, said: (String) -> Unit) {
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val copy = copier(said)
    var adding by remember { mutableStateOf("") }
    var makeLink by remember { mutableStateOf(false) }
    if (makeLink) LinkCodeDialog(node, said) { makeLink = false }
    val devices = strings(s.optJSONObject("settings")?.optJSONArray("devices"))
    val known = nodes(s).associateBy { it.optString("address") }
    val links = s.optJSONObject("links") ?: JSONObject()
    val pending = (links.optJSONArray("pending") ?: JSONArray()).let { a -> (0 until a.length()).map { a.getJSONObject(it) } }
    LazyColumn(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item {
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    Text("Pair a device", style = MaterialTheme.typography.titleMedium)
                    Text("Show a pairing code on one device and scan it with the other: they're each other's devices then. A server shows its code with `vork link-code`.", style = MaterialTheme.typography.bodySmall)
                    Row(Modifier.padding(top = 8.dp)) {
                        Button(onClick = { makeLink = true }) { Text("Show a code") }
                        Spacer(Modifier.width(8.dp))
                        OutlinedButton(onClick = { (context as? MainActivity)?.scanCode() }) { Text("Scan a code") }
                    }
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        OutlinedTextField(adding, { adding = it.trim() }, label = { Text("Or paste a link (vork-link:…)") }, singleLine = true, modifier = Modifier.weight(1f))
                        Spacer(Modifier.width(8.dp))
                        Button(enabled = parseLink(adding) != null, onClick = {
                            val (address, code, _) = parseLink(adding)!!
                            val addrs = parseLink(adding)!!.addrs
                            scope.launch {
                                val r = io { node.link(address, code, addrs) }
                                said(r.optString("error").ifEmpty { if (code.isEmpty()) "Asked: linked once its owner allows it" else "Linking…" })
                                adding = ""
                            }
                        }) { Text("Link") }
                    }
                }
            }
        }
        if (pending.isNotEmpty()) {
            item { Text("Waiting for the other side (${pending.size})", style = MaterialTheme.typography.titleSmall) }
            items(pending) { p ->
                val address = p.optString("address")
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(known[address]?.let { displayName(it) } ?: address.take(8), modifier = Modifier.weight(1f))
                    TextButton(onClick = { scope.launch { said(io { node.unlink(address) }.optString("error").ifEmpty { "Cancelled" }) } }) { Text("Cancel") }
                }
            }
        }
        item { Text("Your devices (${devices.size})", style = MaterialTheme.typography.titleSmall) }
        if (devices.isEmpty()) item { Text("None yet.", style = MaterialTheme.typography.bodySmall) }
        items(devices) { address ->
            val n = known[address]
            val exitVia = s.optJSONObject("settings")?.optString("exit_via").orEmpty()
            val unlink: @Composable () -> Unit = {
                Row {
                    // An exit node (ADR-0026): this phone's internet traffic goes out through it.
                    if (n?.optBoolean("exit") == true || exitVia == address) {
                        val using = exitVia == address
                        TextButton(onClick = {
                            scope.launch {
                                val r = io { node.set("exit_via", if (using) "" else address) }
                                said(r.optString("error").ifEmpty { if (using) "Internet goes out directly again" else "Internet goes out through ${n?.let { displayName(it) } ?: address.take(8)}" })
                            }
                        }) { Text(if (using) "Stop exit node" else "Use as exit node") }
                    }
                    TextButton(onClick = { scope.launch { said(io { node.unlink(address) }.optString("error").ifEmpty { "Unlinked (the other device is told)" }) } }) { Text("Unlink") }
                }
            }
            if (n != null) {
                NodeCard(n, copy, unlink)
            } else {
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(16.dp)) {
                        Text(address.take(8), style = MaterialTheme.typography.titleMedium)
                        Text("Away: not heard now", style = MaterialTheme.typography.bodySmall)
                        val ip = "fd" + address.take(2) + ":" + address.substring(2, 30).chunked(4).joinToString(":")
                        AddressLines(ip, address, address.take(8), copy, derivedIpv4(address))
                        unlink()
                    }
                }
            }
        }
    }
}

/** Makes a link with Resilio's choices, then shows it as a QR code and text to copy or send. */
@Composable
fun LinkCodeDialog(node: Node, said: (String) -> Unit, close: () -> Unit) {
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val copy = copier(said)
    var valid by remember { mutableStateOf(600L) }
    var many by remember { mutableStateOf(false) }
    var approve by remember { mutableStateOf(false) }
    var made by remember { mutableStateOf<JSONObject?>(null) }
    var options by remember { mutableStateOf(false) }
    // Made at once: one device, 10 minutes, linked without asking (the code is the consent).
    LaunchedEffect(Unit) {
        val r = io { node.linkCode(valid, 1, false) }
        if (r.has("error")) { said(r.optString("error")); close() } else made = r
    }
    androidx.compose.material3.AlertDialog(
        onDismissRequest = close,
        title = { Text(if (options) "A pairing code" else "Scan it with the other device") },
        text = {
            val m = made
            if (options || m == null) {
                Column {
                    Text("Valid for", style = MaterialTheme.typography.labelLarge)
                    Row {
                        for ((label, secs) in listOf("10 min" to 600L, "1 day" to 86_400L, "3 days" to 259_200L, "7 days" to 604_800L)) {
                            TextButton(onClick = { valid = secs }) { Text(if (valid == secs) "• $label" else label) }
                        }
                    }
                    Toggle("Any number of devices (else one)", many) { many = it }
                    Toggle("Ask me for each device", approve) { approve = it }
                }
            } else {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    QrCode(m.optJSONArray("qr") ?: JSONArray())
                    Text(m.optString("uri"), style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace))
                    Text(
                        "Scan it with the other phone's camera, or send it. Valid ${if (valid >= 86_400) "${valid / 86_400} day(s)" else "${valid / 60} min"}, " +
                            (if (many) "for any number of devices" else "for one device") + if (approve) "; you allow each one." else ".",
                        style = MaterialTheme.typography.bodySmall,
                    )
                    Row {
                        TextButton(onClick = { copy("Link", m.optString("uri")) }) { Text("Copy") }
                        TextButton(onClick = {
                            val send = android.content.Intent(android.content.Intent.ACTION_SEND).setType("text/plain").putExtra(android.content.Intent.EXTRA_TEXT, m.optString("uri"))
                            context.startActivity(android.content.Intent.createChooser(send, "Send the link"))
                        }) { Text("Send") }
                    }
                }
            }
        },
        confirmButton = {
            if (options) {
                TextButton(onClick = {
                    scope.launch {
                        val r = io { node.linkCode(valid, if (many) 0 else 1, approve) }
                        if (r.has("error")) { said(r.optString("error")); close() } else { made = r; options = false }
                    }
                }) { Text("Make the code") }
            } else {
                TextButton(onClick = close) { Text("Done") }
            }
        },
        dismissButton = { if (!options) TextButton(onClick = { options = true }) { Text("Options") } },
    )
}

/** A QR code from the node's rows of 0 and 1. */
@Composable
fun QrCode(rows: JSONArray) {
    val n = rows.length()
    if (n == 0) return
    androidx.compose.foundation.Canvas(Modifier.padding(8.dp).width(240.dp).height(240.dp)) {
        val quiet = 4
        val cell = size.width / (n + 2 * quiet)
        drawRect(androidx.compose.ui.graphics.Color.White)
        for (y in 0 until n) {
            val row = rows.getString(y)
            for (x in row.indices) {
                if (row[x] == '1') {
                    drawRect(
                        androidx.compose.ui.graphics.Color.Black,
                        topLeft = androidx.compose.ui.geometry.Offset((x + quiet) * cell, (y + quiet) * cell),
                        size = androidx.compose.ui.geometry.Size(cell + 0.5f, cell + 0.5f),
                    )
                }
            }
        }
    }
}

// ---- Settings ---------------------------------------------------------------------------

@Composable
fun SettingsTab(node: Node, s: JSONObject, said: (String) -> Unit) {
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val settings = s.optJSONObject("settings") ?: JSONObject()
    var name by remember(s.optString("name")) { mutableStateOf(s.optString("name")) }
    var peer by remember { mutableStateOf("") }
    var showLog by remember { mutableStateOf(false) }
    val copyLog = copier(said)
    val small = MaterialTheme.typography.bodySmall
    // A change that needs a restart says so; the others apply at once.
    val apply: (JSONObject, String) -> Unit = { r, done ->
        said(r.optString("error").ifEmpty { if (r.optBoolean("restart")) "$done (after a restart)" else done })
        if (r.optBoolean("restart")) NodeService.restart(context)
    }
    LazyColumn(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item {
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    Text("Name", style = MaterialTheme.typography.titleMedium)
                    Text("How other nodes see this one.", style = small)
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        OutlinedTextField(name, { name = it }, singleLine = true, modifier = Modifier.weight(1f))
                        Spacer(Modifier.width(8.dp))
                        Button(enabled = name.isNotBlank() && name != s.optString("name"), onClick = {
                            scope.launch { apply(io { node.set("name", name.trim()) }, "Named ${name.trim()}") }
                        }) { Text("Save") }
                    }
                }
            }
        }
        item {
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    Text("Connections", style = MaterialTheme.typography.titleMedium)
                    Text("Other nodes this one dials by address (host:port). One is enough to join; the rest is learned. Nodes on the same Wi-Fi need none.", style = small)
                    for (p in strings(settings.optJSONArray("peers"))) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(p, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
                            TextButton(onClick = { scope.launch { apply(io { node.removePeer(p) }, "Removed $p") } }) { Text("Remove") }
                        }
                    }
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        OutlinedTextField(peer, { peer = it.trim() }, label = { Text("host:port") }, singleLine = true, modifier = Modifier.weight(1f))
                        Spacer(Modifier.width(8.dp))
                        Button(enabled = peer.isNotBlank(), onClick = {
                            val at = if (peer.contains(':')) peer else "$peer:51820"
                            scope.launch {
                                apply(io { node.addPeer(at) }, "Connecting to $at")
                                peer = ""
                            }
                        }) { Text("Add") }
                    }
                }
            }
        }
        item {
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    Text("Network", style = MaterialTheme.typography.titleMedium)
                    Toggle("Find nodes on this network (mDNS)", settings.optBoolean("lan", true)) { on -> scope.launch { apply(io { node.set("lan", on) }, "Saved") } }
                    Toggle("Ask the router for a port (UPnP)", settings.optBoolean("map_port", true)) { on -> scope.launch { apply(io { node.set("map_port", on) }, "Saved") } }
                    Toggle("Direct links with any node (shows them this phone's address)", settings.optBoolean("direct")) { on -> scope.launch { apply(io { node.set("direct", on) }, "Saved") } }
                    Toggle("Announce this node", settings.optBoolean("announce", true)) { on -> scope.launch { apply(io { node.set("announce", on) }, "Saved") } }
                    Text("UDP port ${settings.optInt("port")}", style = small)
                }
            }
        }
        item {
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    Text("Node", style = MaterialTheme.typography.titleMedium)
                    Text("Address ${s.optString("address")}", style = small.copy(fontFamily = FontFamily.Monospace))
                    if (showLog) {
                        val lines = remember(s) { runCatching { node.log.readLines().takeLast(80) }.getOrDefault(emptyList()) }
                        androidx.compose.material3.AlertDialog(
                            onDismissRequest = { showLog = false },
                            title = { Text("The node's log") },
                            text = {
                                LazyColumn(Modifier.height(420.dp)) {
                                    items(lines) { Text(it, style = small.copy(fontFamily = FontFamily.Monospace)) }
                                }
                            },
                            confirmButton = {
                                TextButton(onClick = { copyLog("Log", lines.joinToString("\n")); showLog = false }) { Text("Copy") }
                            },
                            dismissButton = { TextButton(onClick = { showLog = false }) { Text("Close") } },
                        )
                    }
                    Row {
                        OutlinedButton(onClick = { showLog = true }) { Text("Log") }
                        Spacer(Modifier.width(8.dp))
                        OutlinedButton(onClick = { NodeService.restart(context); said("Restarting the node") }) { Text("Restart") }
                        Spacer(Modifier.width(8.dp))
                        OutlinedButton(onClick = { NodeService.stop(context); said("The node is stopped until the app is opened again") }) { Text("Stop") }
                    }
                }
            }
        }
    }
}

@Composable
fun Toggle(label: String, on: Boolean, set: (Boolean) -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
        Text(label, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
        Switch(checked = on, onCheckedChange = set)
    }
}
