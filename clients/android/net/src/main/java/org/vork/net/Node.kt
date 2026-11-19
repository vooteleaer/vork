package org.vork.net

import android.content.Context
import android.net.LocalSocket
import android.net.LocalSocketAddress
import org.json.JSONObject
import java.io.File

/**
 * The node this app runs: its files, and its control socket (one JSON
 * request per line, one answer per line). Everything the app shows and
 * changes goes through `status` and the settings commands; the node keeps
 * its own `vork.toml`.
 */
class Node(context: Context) {
    val dir = File(context.filesDir, "node")
    val config = File(dir, "vork.toml")
    val state = File(dir, "state")
    val control = File(state, "control.sock")
    val tunSock = File(state, "tun.sock")
    val program = File(context.applicationInfo.nativeLibraryDir, "libvork.so")
    val log = File(dir, "node.log")

    /** One request; the node's answer, or {"error": …} if it doesn't answer. */
    fun ask(request: JSONObject): JSONObject = try {
        LocalSocket().use { s ->
            s.connect(LocalSocketAddress(control.absolutePath, LocalSocketAddress.Namespace.FILESYSTEM))
            s.soTimeout = 5_000
            s.outputStream.write((request.toString() + "\n").toByteArray())
            s.outputStream.flush()
            val line = s.inputStream.bufferedReader().readLine() ?: return JSONObject().put("error", "no answer")
            JSONObject(line)
        }
    } catch (e: Exception) {
        JSONObject().put("error", "the vØrk node isn't running")
    }

    fun status(): JSONObject = ask(JSONObject().put("cmd", "status"))

    fun set(key: String, value: Any): JSONObject = ask(JSONObject().put("cmd", "set").put("key", key).put("value", value))
    fun addPeer(addr: String): JSONObject = ask(JSONObject().put("cmd", "peer_add").put("addr", addr))
    fun removePeer(addr: String): JSONObject = ask(JSONObject().put("cmd", "peer_remove").put("addr", addr))
    fun addDevice(address: String): JSONObject = ask(JSONObject().put("cmd", "device_add").put("address", address))
    fun removeDevice(address: String): JSONObject = ask(JSONObject().put("cmd", "device_remove").put("address", address))
    // Linking devices (vØrk ADR-0024): asked, allowed or dismissed by the other side's owner.
    fun link(address: String, code: String = ""): JSONObject = ask(JSONObject().put("cmd", "link").put("address", address).put("code", code))
    fun allow(address: String): JSONObject = ask(JSONObject().put("cmd", "link_allow").put("address", address))
    fun dismiss(address: String): JSONObject = ask(JSONObject().put("cmd", "link_dismiss").put("address", address))
    fun unlink(address: String): JSONObject = ask(JSONObject().put("cmd", "unlink").put("address", address))
    /** A link (and its QR code) as Resilio's: valid this long, for this many devices (0: any), each allowed or not. */
    fun linkCode(validS: Long, uses: Int, approve: Boolean): JSONObject =
        ask(JSONObject().put("cmd", "link_code").put("valid_s", validS).put("uses", uses).put("approve", approve))

    /** The node lets go of the interface (the VPN is off). */
    fun interfaceOff(): JSONObject = ask(JSONObject().put("cmd", "interface_off"))
    /** The network's own DNS servers: the node passes names outside `.vork` on to them. */
    fun dnsUpstream(servers: List<String>): JSONObject =
        ask(JSONObject().put("cmd", "dns_upstream").put("servers", org.json.JSONArray(servers)))
}
