package org.vork.desktop

import android.Manifest
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
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
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.List
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
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
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.foundation.isSystemInDarkTheme
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
 * vork on Android: this device (the vØrk node and its connections,
 * storage, libraries), the announces, and the settings. The same three views as the
 * desktop utility, over the same core.
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Core.ready(this)
        val wanted = mutableListOf(
            Manifest.permission.READ_CONTACTS, Manifest.permission.WRITE_CONTACTS,
            Manifest.permission.READ_CALENDAR, Manifest.permission.WRITE_CALENDAR,
        )
        if (Build.VERSION.SDK_INT >= 33) wanted += Manifest.permission.POST_NOTIFICATIONS
        val missing = wanted.filter { checkSelfPermission(it) != android.content.pm.PackageManager.PERMISSION_GRANTED }
        if (missing.isNotEmpty()) requestPermissions(missing.toTypedArray(), 1)
        sync()
        VorkService.start(this)
        opened(intent)
        setContent {
            MaterialTheme(colorScheme = if (isSystemInDarkTheme()) darkColorScheme() else lightColorScheme()) {
                App()
            }
        }
    }

    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent)
        opened(intent)
    }

    /**
     * Turns on the interface for apps: the system's VPN consent first, if
     * it hasn't been given (the plain activity-result route: the Compose
     * launcher gave no sign of anything on a phone).
     */
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

    private fun toast(text: String) = android.widget.Toast.makeText(this, text, android.widget.Toast.LENGTH_LONG).show()

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

    /** A vork: link opened elsewhere: the join dialog shows it (nothing is joined until the user says so). */
    private fun opened(intent: android.content.Intent?) {
        val link = intent?.dataString ?: return
        if (link.startsWith("vork:")) LinkOpened.link = link
    }

    private fun allowed(p: String) = checkSelfPermission(p) == android.content.pm.PackageManager.PERMISSION_GRANTED

    /** vork contacts and calendars in the system's own: the account, and a sync now (what's allowed). */
    private fun sync() {
        val contacts = allowed(Manifest.permission.WRITE_CONTACTS)
        val calendar = allowed(Manifest.permission.WRITE_CALENDAR)
        if (!contacts && !calendar) return
        Accounts.ensure(this)
        if (contacts) Accounts.syncContacts(this)
        if (calendar) Accounts.syncCalendars()
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        sync()
    }
}

/** Calls the core off the main thread. */
suspend fun <T> core(work: () -> T): T = withContext(Dispatchers.IO) { work() }

fun human(n: Long): String {
    val units = listOf("B", "KB", "MB", "GB", "TB")
    var v = n.toDouble()
    var i = 0
    while (v >= 1024 && i < units.size - 1) { v /= 1024; i++ }
    return if (i == 0) "$n B" else String.format("%.1f %s", v, units[i])
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun App() {
    var tab by remember { mutableIntStateOf(0) }
    var status by remember { mutableStateOf(JSONObject()) }
    var network by remember { mutableStateOf(JSONObject()) }
    var tick by remember { mutableIntStateOf(0) }
    var note by remember { mutableStateOf("") }
    // A vork: link opened elsewhere, wherever the app is.
    LinkOpened.link?.let { given -> JoinLinkDialog({ note = it }, given) { LinkOpened.link = null } }
    LaunchedEffect(tick) {
        while (true) {
            status = JSONObject(core { Core.status() })
            network = JSONObject(core { Core.network() })
            // Often while it starts, so its steps and progress move.
            delay(if (status.optBoolean("up")) 5_000 else 1_000)
        }
    }
    val refresh = { tick++ }
    Scaffold(
        topBar = { TopAppBar(title = { Text(listOf("vØrk", "Libraries", "Announces", "Settings")[tab]) }) },
        bottomBar = {
            NavigationBar {
                NavigationBarItem(selected = tab == 0, onClick = { tab = 0 }, icon = { Icon(Icons.Filled.Home, null) }, label = { Text("Network") })
                NavigationBarItem(selected = tab == 1, onClick = { tab = 1 }, icon = { Icon(Icons.Filled.List, null) }, label = { Text("Libraries") })
                NavigationBarItem(selected = tab == 2, onClick = { tab = 2 }, icon = { Icon(Icons.Filled.Share, null) }, label = { Text("Announces") })
                NavigationBarItem(selected = tab == 3, onClick = { tab = 3 }, icon = { Icon(Icons.Filled.Settings, null) }, label = { Text("Settings") })
            }
        },
    ) { inner ->
        Column(Modifier.padding(inner).fillMaxSize()) {
            if (note.isNotEmpty()) {
                Text(note, Modifier.padding(horizontal = 16.dp, vertical = 4.dp), style = MaterialTheme.typography.bodySmall)
            }
            when (tab) {
                0 -> NetworkTab(network) { note = it; refresh() }
                1 -> Overview(status, network) { note = it; refresh() }
                2 -> Announces(status) { note = it; refresh() }
                else -> SettingsView { note = it; refresh() }
            }
        }
    }
}

@Composable
fun Overview(s: JSONObject, network: JSONObject, said: (String) -> Unit) {
    val context = LocalContext.current
    var newLibrary by remember { mutableStateOf(false) }
    var joinLink by remember { mutableStateOf(false) }
    if (newLibrary) NewLibraryDialog(said) { newLibrary = false }
    if (joinLink) JoinLinkDialog(said) { joinLink = false }
    LazyColumn(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item {
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    DaemonState(s, network.optBoolean("running"))
                    s.optJSONObject("usage")?.let { u ->
                        Spacer(Modifier.padding(4.dp))
                        Text("Own data: ${human(u.optLong("pinned"))} of ${human(u.optLong("store_limit"))}")
                        Text("Held for others: ${human(u.optLong("leased"))}")
                        Text("Cache: ${human(u.optLong("cache"))} of ${human(u.optLong("cache_max"))}")
                        Text("Free on the phone: ${human(u.optLong("disk_available"))}")
                    }
                    Row(Modifier.padding(top = 12.dp)) {
                        Button(onClick = { VorkService.start(context); said("Starting vork") }) { Text("Start") }
                        Spacer(Modifier.width(8.dp))
                        OutlinedButton(onClick = { VorkService.restartDaemon(context); said("Restarting vork") }) { Text("Restart") }
                        Spacer(Modifier.width(8.dp))
                        OutlinedButton(onClick = { VorkService.stop(context); said("vork stopped") }) { Text("Stop") }
                    }
                }
            }
        }
        val libraries = s.optJSONArray("libraries") ?: JSONArray()
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Libraries", style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f))
                TextButton(onClick = { joinLink = true }) { Text("Join with link") }
                IconButton(onClick = { newLibrary = true }) { Icon(Icons.Filled.Add, "New library") }
            }
        }
        if (libraries.length() == 0) {
            item { Text("None yet.", style = MaterialTheme.typography.bodySmall) }
        }
        items((0 until libraries.length()).map { libraries.getJSONObject(it) }) { l -> LibraryCard(l, said) }
    }
}

/** The public libraries announced on the network, to join. */
@Composable
fun Announces(s: JSONObject, said: (String) -> Unit) {
    val scope = rememberCoroutineScope()
    var heard by remember { mutableStateOf(JSONArray()) }
    var heardError by remember { mutableStateOf("") }
    LaunchedEffect(s) {
        val answer = core { Core.heard() }
        if (answer.startsWith("[")) {
            heard = JSONArray(answer)
            heardError = ""
        } else {
            heardError = JSONObject(answer).optString("error", answer)
        }
    }
    LazyColumn(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (heardError.isNotEmpty()) {
            item { Text(heardError, style = MaterialTheme.typography.bodySmall) }
        } else if (heard.length() == 0) {
            item { Text("No public library heard yet. Announces come every few minutes once connected.", style = MaterialTheme.typography.bodySmall) }
        }
        items((0 until heard.length()).map { heard.getJSONObject(it) }) { h ->
            Card(Modifier.fillMaxWidth()) {
                Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(h.optString("name"), style = MaterialTheme.typography.titleMedium)
                        if (h.optString("about").isNotEmpty()) Text(h.optString("about"), style = MaterialTheme.typography.bodySmall)
                        Text("${h.optInt("hops")} hop(s) away", style = MaterialTheme.typography.bodySmall)
                    }
                    when {
                        !h.isNull("here") -> Text("Joined", style = MaterialTheme.typography.labelLarge)
                        !h.optBoolean("readable", true) -> Text("Needs a newer vork", style = MaterialTheme.typography.labelSmall)
                        else -> Button(onClick = {
                            scope.launch { said(core { Core.join(h.optString("address")) }.ifEmpty { "Joined ${h.optString("name")}" }) }
                        }) { Text("Join") }
                    }
                }
            }
        }
    }
}

/**
 * The network, as Tailscale shows a tailnet: this device and every node it
 * knows, each with its name, its `fd…` address and its vØrk address to
 * copy (a tap on a row copies the address), and how it's reached; then the
 * connections and the interface for apps.
 */
@Composable
fun NetworkTab(n: JSONObject, said: (String) -> Unit) {
    val clipboard = LocalClipboardManager.current
    val copy = { label: String, text: String ->
        clipboard.setText(AnnotatedString(text))
        said("$label copied: $text")
    }
    val nodes = n.optJSONArray("nodes") ?: JSONArray()
    val mine = (0 until nodes.length()).map { nodes.getJSONObject(it) }
    LazyColumn(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item {
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    val name = n.optString("name").ifEmpty { "This device" }
                    Text("$name (this device)", style = MaterialTheme.typography.titleMedium)
                    if (!n.optBoolean("running")) {
                        Text(n.optString("error").ifEmpty { "The vØrk node isn't running" }, style = MaterialTheme.typography.bodySmall)
                    } else {
                        NodeLines(n.optString("node_ip"), n.optString("address"), name, copy)
                    }
                }
            }
        }
        item {
            Text("Nodes (${mine.size})", style = MaterialTheme.typography.titleSmall)
        }
        if (mine.isEmpty()) {
            item { Text("None known yet: nodes on the same Wi-Fi appear by themselves; others once connected (below).", style = MaterialTheme.typography.bodySmall) }
        }
        items(mine) { node ->
            Card(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    val address = node.optString("address")
                    val name = node.optString("name").takeIf { !node.isNull("name") && it.isNotEmpty() } ?: address.take(8)
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(name, style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                        val how = when {
                            node.optBoolean("direct") -> "direct"
                            else -> "${node.optInt("hops")} hop(s), relayed"
                        }
                        Text(how + if (node.optBoolean("device")) " · your device" else "", style = MaterialTheme.typography.labelSmall)
                    }
                    NodeLines(node.optString("ip"), address, name, copy)
                }
            }
        }
        item { Network(n, said) }
    }
}

/** A node's addresses, each with Copy: its `fd…` address, its DNS name, its vØrk address. */
@Composable
fun NodeLines(ip: String, address: String, name: String, copy: (String, String) -> Unit) {
    val mono = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace)
    val dns = dnsName(name, address)
    for ((label, value) in listOf("IP" to ip, "DNS" to dns, "Address" to address)) {
        if (value.isEmpty()) continue
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(label, style = MaterialTheme.typography.labelSmall, modifier = Modifier.width(56.dp))
            Text(value, style = mono, modifier = Modifier.weight(1f))
            TextButton(onClick = { copy(label, value) }) { Text("Copy") }
        }
    }
}

/** The DNS name a node will have (vØrk ADR-0018): its name, made a label, under `.vork`. */
fun dnsName(name: String, address: String): String {
    val label = name.lowercase().replace(Regex("[^a-z0-9-]+"), "-").trim('-').take(40)
    return (label.ifEmpty { address.take(8) }) + ".vork"
}

/**
 * What vork is doing: running, starting (the step vorkd is at, with a
 * progress bar), waiting for the vØrk node, stopped by itself (why), or not
 * started. Never a bare "not running" while it starts.
 */
@Composable
fun DaemonState(s: JSONObject, nodeUp: Boolean) {
    val starting = s.optJSONObject("starting")
    val title = MaterialTheme.typography.titleMedium
    val small = MaterialTheme.typography.bodySmall
    when {
        s.optBoolean("up") -> {
            Text("vork is running", style = title)
            Text("Node ${s.optString("node")}", style = small)
        }
        starting != null || VorkService.daemonAlive -> {
            Text("vork is starting…", style = title)
            val progress = starting?.takeIf { !it.isNull("progress") }?.optDouble("progress")
            Spacer(Modifier.padding(4.dp))
            if (progress != null) {
                LinearProgressIndicator(progress = { progress.toFloat() }, modifier = Modifier.fillMaxWidth())
            } else {
                LinearProgressIndicator(Modifier.fillMaxWidth())
            }
            Spacer(Modifier.padding(4.dp))
            val step = starting?.optString("text")?.takeIf { it.isNotEmpty() } ?: "starting the daemon"
            val since = starting?.optLong("since_ms") ?: 0L
            val took = if (since > 0) (System.currentTimeMillis() - since) / 1000 else 0L
            Text(step.replaceFirstChar { it.uppercase() } + if (took >= 10) " (${took / 60} min ${took % 60} s)" else "", style = small)
            if (starting?.optString("step")?.startsWith("checking") == true) {
                Text("vork wasn't stopped cleanly last time, so it checks its store first. On a large store this takes a few minutes.", style = small)
            }
        }
        VorkService.waitingForNode || !nodeUp -> {
            Text("Waiting for the vØrk node", style = title)
            Text("Storage starts as soon as the node is running.", style = small)
        }
        VorkService.lastExit != null -> {
            Text("vork stopped", style = title, color = MaterialTheme.colorScheme.error)
            Text(VorkService.lastExit ?: "", style = small)
            Text("It's started again in a few seconds; Restart tries at once.", style = small)
        }
        else -> {
            Text("vork isn't running", style = title)
            Text("Start starts it.", style = small)
        }
    }
}

/**
 * The app's vØrk node and its connections: other nodes by address
 * (`host:port`, their key asked for on first contact), kept in the node's
 * vork.toml and dialed at once.
 */
@Composable
fun Network(n: JSONObject, said: (String) -> Unit) {
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    var address by remember { mutableStateOf("") }
    val small = MaterialTheme.typography.bodySmall
    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp)) {
            Text("Connections", style = MaterialTheme.typography.titleMedium)
            if (n.optBoolean("running")) {
                Text("The vØrk node is running: ${n.optString("address")}", style = small)
            } else {
                Text(n.optString("error").ifEmpty { "The vØrk node isn't running" }, style = small)
            }
            val connections = n.optJSONArray("connections") ?: JSONArray()
            val others = n.optJSONArray("others") ?: JSONArray()
            for (c in (0 until connections.length()).map { connections.getJSONObject(it) }) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    val state = when {
                        !c.optBoolean("enabled", true) -> "off"
                        c.optBoolean("up") -> "connected"
                        else -> "not connected"
                    }
                    Text("${c.optString("name")} · $state", Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
                    TextButton(onClick = {
                        scope.launch { said(core { Core.removeConnection(c.optString("name")) }.ifEmpty { "Removed ${c.optString("name")}" }) }
                    }) { Text("Remove") }
                }
            }
            val ip = n.optString("ip").takeIf { !n.isNull("ip") && it.isNotEmpty() }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    if (ip != null) "Apps reach this phone at $ip" else "Apps on this phone don't reach vØrk addresses",
                    Modifier.weight(1f),
                    style = small,
                )
                if (ip == null) {
                    TextButton(onClick = {
                        val activity = context as? MainActivity
                        if (activity != null) activity.turnOnVpn() else said("Open vØrk from its icon to turn this on")
                    }) { Text("Turn on") }
                } else {
                    TextButton(onClick = { VorkVpn.stop(context); said("Apps no longer reach vØrk addresses") }) { Text("Turn off") }
                }
            }
            VorkVpn.state?.let { if (ip == null) Text("Interface: $it", style = small) }
            if (n.optInt("reached_by") > 0) {
                Text("Reachable directly: ${n.optInt("reached_by")} node(s) reached this phone", style = small)
            }
            for (o in (0 until others.length()).map { others.getJSONObject(it) }) {
                Text("${o.optString("name")} · ${if (o.optBoolean("up")) "connected" else "not connected"} (found by itself)", style = small)
            }
            Row(Modifier.padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                OutlinedTextField(
                    value = address,
                    onValueChange = { address = it },
                    label = { Text("Another node: host:port") },
                    singleLine = true,
                    modifier = Modifier.weight(1f),
                )
                Spacer(Modifier.width(8.dp))
                Button(enabled = address.isNotBlank(), onClick = {
                    val at = address.trim()
                    val host = at.substringBeforeLast(':', at)
                    val port = at.substringAfterLast(':', "51820").toIntOrNull() ?: 51820
                    scope.launch {
                        said(core { Core.addConnection(host, port) }.ifEmpty { "Connecting to $host:$port" })
                        address = ""
                    }
                }) { Text("Add") }
            }
        }
    }
}

@Composable
fun SettingsView(said: (String) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var list by remember { mutableStateOf(JSONArray()) }
    val values = remember { mutableStateMapOf<String, String>() }
    LaunchedEffect(Unit) {
        list = JSONArray(core { Core.settings() })
        for (i in 0 until list.length()) {
            val s = list.getJSONObject(i)
            values["${s.getString("section")}.${s.getString("key")}"] = if (s.isNull("value")) "" else s.getString("value")
        }
    }
    LazyColumn(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item { RecoveryCard(said) }
        item { Text("This node's vork.toml, filled with the defaults this device gets until you change them.", style = MaterialTheme.typography.bodySmall) }
        items((0 until list.length()).map { list.getJSONObject(it) }) { s ->
            val id = "${s.getString("section")}.${s.getString("key")}"
            if (s.optBoolean("switch")) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(s.getString("what"), Modifier.weight(1f))
                    Switch(checked = values[id] == "true", onCheckedChange = { values[id] = if (it) "true" else "" })
                }
            } else {
                val unit = s.optString("unit", "")
                OutlinedTextField(values[id] ?: "", { values[id] = it }, label = { Text(s.getString("what")) }, singleLine = true, modifier = Modifier.fillMaxWidth(),
                    suffix = if (unit.isNotEmpty()) ({ Text(unit) }) else null)
            }
        }
        item {
            Button(onClick = {
                val json = JSONObject()
                for ((k, v) in values) json.put(k, v)
                scope.launch {
                    val e = core { Core.saveSettings(json.toString()) }
                    if (e.isEmpty()) VorkService.restartDaemon(context)
                    said(e.ifEmpty { "Settings saved; vork restarted" })
                }
            }) { Text("Save and restart vork") }
        }
    }
}
