package org.vork.net

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** The node comes up with the phone. */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED) NodeService.start(context)
    }
}
