package org.vork.net

import android.content.Context
import android.content.Intent
import android.net.ConnectivityManager
import android.net.LocalSocket
import android.net.NetworkCapabilities
import android.net.LocalSocketAddress
import android.net.VpnService
import android.os.ParcelFileDescriptor
import java.io.File
import java.net.InetAddress

/**
 * Ordinary apps reach vØrk nodes at their `fd…` addresses (vØrk M2): an
 * interface with this node's address and `fd00::/8` routed to it, nothing
 * else (the rest of the phone's traffic doesn't touch it). The node runs in
 * its own process, so the interface is handed to it over
 * `node/state/tun.sock` (the node has `tun = "fd"`); handed again whenever
 * the node comes back without it.
 *
 * Names (ADR-0025): the phone's DNS server is the node, at `fd00::53` inside
 * that route. It answers `.vork` itself and passes the rest on to the
 * network's own servers, which this service tells it (again on every change
 * of network). The app stays out of its own VPN, so the node's traffic, those
 * questions included, goes out as before.
 */
class VorkVpn : VpnService() {
    private var iface: ParcelFileDescriptor? = null
    @Volatile private var running = false
    private var keeper: Thread? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopSelf()
            return START_NOT_STICKY
        }
        if (running) return START_STICKY
        state = "starting"
        running = true
        keeper = Thread(::keep, "vork-vpn").apply { start() }
        return START_STICKY
    }

    override fun onRevoke() {
        // The user turned it off in the system's VPN settings.
        getSharedPreferences("vork", MODE_PRIVATE).edit().putBoolean("vpn", false).apply()
        stopSelf()
    }

    override fun onDestroy() {
        state = null
        running = false
        keeper?.interrupt()
        iface?.close()
        iface = null
        // The node holds a copy of the interface: it lets go too, or the VPN stays.
        val node = Node(this)
        Thread { node.interfaceOff() }.start()
        super.onDestroy()
    }

    /** Makes the interface once the node's address is known; hands it over while the node lacks it. */
    private fun keep() {
        val node = Node(this)
        val sock = node.tunSock
        while (running) {
            try {
                val n = node.status()
                val address = n.optString("address")
                val before = state
                when {
                    n.has("error") || address.length != 32 -> state = "waiting for the vØrk node"
                    else -> {
                        val pfd = iface ?: establish(address)?.also { iface = it }
                        when {
                            pfd == null -> state = "Android didn't make the interface (is another VPN always on?)"
                            !n.isNull("interface") -> state = null
                            !sock.exists() -> state = "the node isn't waiting for an interface (restart the app)"
                            else -> {
                                state = "handing the interface to the node"
                                hand(pfd, sock)
                            }
                        }
                    }
                }
                if (state == null) keepDns(node, n)
                if (state != before) say(state ?: "Apps reach vØrk addresses now")
                Thread.sleep(if (state == null) 5_000 else 2_000)
            } catch (e: InterruptedException) {
                return
            } catch (e: Exception) {
                state = "error: ${e.message ?: e.javaClass.simpleName}"
                android.util.Log.w("vork", "vpn", e)
                say("vØrk interface: ${state}")
                try { Thread.sleep(10_000) } catch (stop: InterruptedException) { return }
            }
        }
    }

    /** Tells the node the network's DNS servers when they aren't what it has (a new network, a restarted node). */
    private fun keepDns(node: Node, status: org.json.JSONObject) {
        val want = upstreamDns()
        val has = status.optJSONObject("dns")?.optJSONArray("upstream")
        val now = (0 until (has?.length() ?: 0)).map { has!!.getString(it) }
        if (now != want.map { if (it.contains(':')) "[$it]:53" else "$it:53" }) node.dnsUpstream(want)
    }

    /** The DNS servers of the network the phone uses under the VPN (link-local ones can't be used from the node). */
    private fun upstreamDns(): List<String> {
        val cm = getSystemService(ConnectivityManager::class.java)
        val networks = listOfNotNull(cm.activeNetwork) + cm.allNetworks.toList()
        for (net in networks.distinct()) {
            val caps = cm.getNetworkCapabilities(net) ?: continue
            if (caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN) || !caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)) continue
            val list = cm.getLinkProperties(net)?.dnsServers.orEmpty()
                .filterNot { it.isLinkLocalAddress }
                .mapNotNull { it.hostAddress?.substringBefore('%') }
            if (list.isNotEmpty()) return list
        }
        // None known: public resolvers, so the phone's names keep working.
        return listOf("1.1.1.1", "9.9.9.9")
    }

    /** Says it on screen too (the service has no window of its own). */
    private fun say(text: String) {
        android.os.Handler(mainLooper).post { android.widget.Toast.makeText(applicationContext, "vØrk: $text", android.widget.Toast.LENGTH_SHORT).show() }
    }

    private fun establish(address: String): ParcelFileDescriptor? {
        // fd ‖ the node's address[0..15]
        val bytes = ByteArray(16)
        bytes[0] = 0xfd.toByte()
        for (i in 0 until 15) bytes[i + 1] = address.substring(i * 2, i * 2 + 2).toInt(16).toByte()
        return Builder()
            .setSession("vØrk")
            .addAddress(InetAddress.getByAddress(bytes), 128)
            .addRoute("fd00::", 8)
            // The node answers names (.vork itself, the rest from the network's servers).
            .addDnsServer(InetAddress.getByName("fd00::53"))
            // The node's own traffic (to other nodes, and those DNS questions) stays outside.
            .addDisallowedApplication(packageName)
            // Without this, Android blocks every family the VPN has no route
            // for: IPv4 (the phone's internet) went dark when it came on.
            .allowFamily(android.system.OsConstants.AF_INET)
            .allowFamily(android.system.OsConstants.AF_INET6)
            .setMtu(1280)
            .establish()
    }

    /** Sends the interface's descriptor to the node (SCM_RIGHTS). */
    private fun hand(pfd: ParcelFileDescriptor, sock: File) {
        LocalSocket().use { s ->
            s.connect(LocalSocketAddress(sock.absolutePath, LocalSocketAddress.Namespace.FILESYSTEM))
            s.setFileDescriptorsForSend(arrayOf(pfd.fileDescriptor))
            s.outputStream.write(1)
            s.outputStream.flush()
            Thread.sleep(200) // the node takes it before the socket closes
        }
    }

    companion object {
        const val ACTION_STOP = "org.vork.net.VPN_STOP"

        /** What it's doing or what went wrong, for the app to show (null: running, or off). */
        @Volatile var state: String? = null

        /** Starts it if the user allowed it before (no dialog needed now). */
        fun startIfAllowed(context: Context) {
            val wanted = context.getSharedPreferences("vork", MODE_PRIVATE).getBoolean("vpn", false)
            if (!wanted) return
            if (prepare(context) != null) {
                state = "Android hasn't allowed the VPN yet: Turn on again"
                return
            }
            context.startService(Intent(context, VorkVpn::class.java))
        }

        fun stop(context: Context) {
            context.getSharedPreferences("vork", MODE_PRIVATE).edit().putBoolean("vpn", false).apply()
            context.startService(Intent(context, VorkVpn::class.java).setAction(ACTION_STOP))
        }
    }
}
