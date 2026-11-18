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
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Search
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
 * Overview (this device, the interface for apps), My devices, Discovery
 * (nodes found and heard), Settings. Like Tailscale, without a server.
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        NodeService.start(this)
        setContent {
            MaterialTheme(colorScheme = if (isSystemInDarkTheme()) darkColorScheme() else lightColorScheme()) {
                App(Node(this))
            }
        }
    }

    /** The system's VPN consent first, if it hasn't been given; then the interface. */
    fun turnOnVpn() {
        try {
            val ask = android.net.VpnService.prepare(this)
            if (ask != null) {
                toast("Android asks whether vØrk may set up a VPN (fd00::/8 only)")
                @Suppress("DEPRECATION")
                startActivityForResult(ask, REQUEST_VPN)
            } else {
                vpnAllowed()
            }
        } catch (e: Exception) {
            toast("Couldn't turn it on: ${e.message ?: e.javaClass.simpleName}")
        }
    }

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
            if (resultCode == RESULT_OK) vpnAllowed() else toast("The VPN wasn't allowed, so apps can't reach vØrk addresses")
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
    Scaffold(
        topBar = { TopAppBar(title = { Text(listOf("vØrk", "My devices", "Discovery", "Settings")[tab]) }) },
        bottomBar = {
            NavigationBar {
                NavigationBarItem(selected = tab == 0, onClick = { tab = 0 }, icon = { Icon(Icons.Filled.Home, null) }, label = { Text("Overview") })
                NavigationBarItem(selected = tab == 1, onClick = { tab = 1 }, icon = { Icon(Icons.Filled.Person, null) }, label = { Text("My devices") })
                NavigationBarItem(selected = tab == 2, onClick = { tab = 2 }, icon = { Icon(Icons.Filled.Search, null) }, label = { Text("Discovery") })
                NavigationBarItem(selected = tab == 3, onClick = { tab = 3 }, icon = { Icon(Icons.Filled.Settings, null) }, label = { Text("Settings") })
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
                2 -> Discovery(node, status, said)
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

/** A node's IP, DNS name and vØrk address, each with Copy. */
@Composable
fun AddressLines(ip: String, address: String, name: String, copy: (String, String) -> Unit) {
    val mono = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace)
    for ((label, value) in listOf("IP" to ip, "DNS" to dnsName(name, address), "Address" to address)) {
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
            AddressLines(n.optString("ip"), n.optString("address"), displayName(n), copy)
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
                        AddressLines(s.optString("ip"), s.optString("address"), name, copy)
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
                    Text("Apps on this phone", style = MaterialTheme.typography.titleMedium)
                    val iface = s.optJSONObject("interface")
                    if (iface != null) {
                        Text("Apps reach vØrk nodes at their IP addresses (fd…): ping, ssh, a browser…", style = small)
                        TextButton(onClick = { VorkVpn.stop(context); said("Apps no longer reach vØrk addresses (fully once the node restarts)") }) { Text("Turn off") }
                    } else {
                        Text("Off: apps don't reach vØrk addresses. On, only fd00::/8 goes through vØrk; the rest of the phone's traffic doesn't.", style = small)
                        VorkVpn.state?.let { Text("Interface: $it", style = small) }
                        Button(onClick = { (context as? MainActivity)?.turnOnVpn() }) { Text("Turn on") }
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

@Composable
fun MyDevices(node: Node, s: JSONObject, said: (String) -> Unit) {
    val scope = rememberCoroutineScope()
    val copy = copier(said)
    var adding by remember { mutableStateOf("") }
    val devices = strings(s.optJSONObject("settings")?.optJSONArray("devices"))
    val known = nodes(s).associateBy { it.optString("address") }
    LazyColumn(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item {
            Text(
                "Your other devices: they get direct links with this one, and are listed here even when away. Add one by its address (vØrk → Overview → Address on it), or from Discovery.",
                style = MaterialTheme.typography.bodySmall,
            )
        }
        if (devices.isEmpty()) item { Text("None yet.", style = MaterialTheme.typography.bodySmall) }
        items(devices) { address ->
            val n = known[address]
            val remove: @Composable () -> Unit = {
                TextButton(onClick = { scope.launch { said(io { node.removeDevice(address) }.optString("error").ifEmpty { "Removed from your devices" }) } }) { Text("Remove") }
            }
            if (n != null) {
                NodeCard(n, copy, remove)
            } else {
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(16.dp)) {
                        Text(address.take(8), style = MaterialTheme.typography.titleMedium)
                        Text("Not heard now: away, or not connected to any node this one knows", style = MaterialTheme.typography.bodySmall)
                        val ip = "fd" + address.take(2) + ":" + address.substring(2, 30).chunked(4).joinToString(":")
                        AddressLines(ip, address, address.take(8), copy)
                        remove()
                    }
                }
            }
        }
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                OutlinedTextField(adding, { adding = it.trim() }, label = { Text("A device's address (32 hex digits)") }, singleLine = true, modifier = Modifier.weight(1f))
                Spacer(Modifier.width(8.dp))
                Button(enabled = adding.length == 32, onClick = {
                    scope.launch {
                        said(io { node.addDevice(adding) }.optString("error").ifEmpty { "Added to your devices" })
                        adding = ""
                    }
                }) { Text("Add") }
            }
        }
    }
}

// ---- Discovery ---------------------------------------------------------------------------

@Composable
fun Discovery(node: Node, s: JSONObject, said: (String) -> Unit) {
    val scope = rememberCoroutineScope()
    val copy = copier(said)
    var query by remember { mutableStateOf("") }
    var showAll by remember { mutableStateOf(false) }
    val devices = strings(s.optJSONObject("settings")?.optJSONArray("devices")).toSet()
    val all = nodes(s).filter { it.optString("address") !in devices }
    val lan = all.filter { it.optBoolean("on_lan") }
    val connected = all.filter { !it.optBoolean("on_lan") && it.optBoolean("direct") }
    val rest = all.filter { !it.optBoolean("on_lan") && !it.optBoolean("direct") }
        .filter { query.isBlank() || displayName(it).contains(query, true) || it.optString("address").startsWith(query.lowercase()) }
    val add: (JSONObject) -> @Composable () -> Unit = { n ->
        {
            TextButton(onClick = { scope.launch { said(io { node.addDevice(n.optString("address")) }.optString("error").ifEmpty { "${displayName(n)} is one of your devices now" }) } }) {
                Text("This is my device")
            }
        }
    }
    LazyColumn(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item { Text("On this network (${lan.size})", style = MaterialTheme.typography.titleSmall) }
        if (lan.isEmpty()) item { Text("No vØrk node found on this Wi-Fi.", style = MaterialTheme.typography.bodySmall) }
        items(lan) { NodeCard(it, copy, add(it)) }
        item { Text("Connected (${connected.size})", style = MaterialTheme.typography.titleSmall) }
        if (connected.isEmpty()) item { Text("No other node linked directly.", style = MaterialTheme.typography.bodySmall) }
        items(connected) { NodeCard(it, copy, add(it)) }
        item {
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.clickable { showAll = !showAll }) {
                Text("Elsewhere on the network (${rest.size})", style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f))
                Text(if (showAll) "Hide" else "Show", style = MaterialTheme.typography.labelLarge)
            }
        }
        if (showAll) {
            item { OutlinedTextField(query, { query = it }, label = { Text("Search by name or address") }, singleLine = true, modifier = Modifier.fillMaxWidth()) }
            items(rest.take(200)) { NodeCard(it, copy, add(it)) }
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
                    Row {
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
