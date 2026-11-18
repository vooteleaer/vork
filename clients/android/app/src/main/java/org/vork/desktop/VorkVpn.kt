package org.vork.desktop

import android.content.Context
import android.content.Intent
import android.net.LocalSocket
import android.net.LocalSocketAddress
import android.net.VpnService
import android.os.ParcelFileDescriptor
import org.json.JSONObject
import java.io.File
import java.net.InetAddress

/**
 * Ordinary apps reach vØrk nodes at their `fd…` addresses (vØrk M2): an
 * interface with this node's address and `fd00::/8` routed to it, nothing
 * else (the rest of the phone's traffic doesn't touch it). The node runs in
 * its own process, so the interface is handed to it over
 * `node/state/tun.sock` (the node has `tun = "fd"`); handed again whenever
 * the node comes back without it.
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
        Core.ready(this)
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
        running = false
        keeper?.interrupt()
        iface?.close()
        iface = null
        super.onDestroy()
    }

    /** Makes the interface once the node's address is known; hands it over while the node lacks it. */
    private fun keep() {
        val sock = File(Core.Paths(this).node, "state/tun.sock")
        while (running) {
            try {
                val n = JSONObject(Core.network())
                val address = n.optString("address")
                if (n.optBoolean("running") && address.length == 32) {
                    val pfd = iface ?: establish(address)?.also { iface = it }
                    if (pfd != null && n.isNull("ip") && sock.exists()) hand(pfd, sock)
                }
                Thread.sleep(5_000)
            } catch (e: InterruptedException) {
                return
            } catch (e: Exception) {
                try { Thread.sleep(10_000) } catch (stop: InterruptedException) { return }
            }
        }
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
        const val ACTION_STOP = "org.vork.desktop.VPN_STOP"

        /** Starts it if the user allowed it before (no dialog needed now). */
        fun startIfAllowed(context: Context) {
            val wanted = context.getSharedPreferences("vork", MODE_PRIVATE).getBoolean("vpn", false)
            if (wanted && prepare(context) == null) {
                context.startService(Intent(context, VorkVpn::class.java))
            }
        }

        fun stop(context: Context) {
            context.getSharedPreferences("vork", MODE_PRIVATE).edit().putBoolean("vpn", false).apply()
            context.startService(Intent(context, VorkVpn::class.java).setAction(ACTION_STOP))
        }
    }
}
