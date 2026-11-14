// WP P1AN1: a document opened offline (PRO1 W7, P1OF's `open_document`) as a
// device of its own: the demo stack (AN28) on a core that opens the document
// where the demo opens the simulated DSPi. Nothing reaches USB or a network:
// every way out is closed as in [DemoCore].
package se.bofr.dspi.app

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import se.bofr.dspi.core.DeviceInfo
import se.bofr.dspi.core.DspiCoreApi
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.ffi.FfiDspiCore

/** The serial the core gives an open document (`offline.rs`). */
internal const val OFFLINE_DOCUMENT_SERIAL = "OFFLINE-DOCUMENT"

/**
 * The bridge finds the document the usual way: [scanDevices] lists it and
 * [selectDevice] opens it on the bridge's FFI thread, so the bridge reads it
 * and takes its baseline as for any device. A document the core cannot read
 * fails the open; the bridge then says it could not be opened.
 */
internal class DocumentCore(private val core: DspiCoreApi, private val text: String, private val fileName: String) :
    DspiCoreApi by DemoCore(core) {
    override fun scanDevices(): List<DeviceInfo> = listOf(DeviceInfo(OFFLINE_DOCUMENT_SERIAL, 0))

    override fun selectDevice(serial: String) {
        if (serial != OFFLINE_DOCUMENT_SERIAL) throw DspiException.Disconnected()
        if (!core.isConnected()) core.openDocument(text, fileName)
    }
}

/** The demo stack on [DocumentCore]: what [AppModeController.openDocument] starts. */
fun startDocumentSession(
    parent: CoroutineScope,
    appVersion: String,
    text: String,
    fileName: String,
    main: CoroutineDispatcher = Dispatchers.Main.immediate,
    core: DspiCoreApi = FfiDspiCore(),
    log: (String) -> Unit = {}
): DemoSession = startDemoSession(parent, appVersion, main = main, core = DocumentCore(core, text, fileName), log = log)
