// WP AN6. macOS: DSPViewModel `inputSourceNames` / `deviceName`, Commands.swift
// `fetchOutputLatency` / `setOutputLatency`; Qt: NamesController, OutputLatencyController.
package se.bofr.dspi.bridge

import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import se.bofr.dspi.core.DspiCoreApi
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.OutputLatency

/** See [DeviceSettings]. */
class DeviceSettingsImpl internal constructor(private val bridge: DspiBridgeImpl) : DeviceSettings {
    private val _names = MutableStateFlow(DeviceNames())
    override val names: StateFlow<DeviceNames> = _names.asStateFlow()
    private val _latency = MutableStateFlow(OutputLatencyState())
    override val latency: StateFlow<OutputLatencyState> = _latency.asStateFlow()
    private val _coreVoltage = MutableStateFlow(CoreVoltageState())
    override val coreVoltage: StateFlow<CoreVoltageState> = _coreVoltage.asStateFlow()

    private var seenGeneration = bridge.deviceGeneration.value

    init {
        bridge.deviceChangedHooks += {
            // Another unit leaving doesn't touch the open device's rows
            if (seenGeneration != generation) {
                seenGeneration = generation
                clear()
            }
            if (bridge.connected) {
                bridge.scope.launch(bridge.main) {
                    refreshNames()
                    probeCoreVoltage()
                }
            }
        }
    }

    /** No device, or another one: nothing of the last one stays. Main thread. */
    internal fun clear() {
        _names.value = DeviceNames()
        _latency.value = OutputLatencyState()
        _coreVoltage.value = CoreVoltageState()
    }

    private val generation: Long get() = bridge.deviceGeneration.value

    // ── Names ──

    /** Reads the core's copy, so it can run on every batch of notifications. */
    internal suspend fun refreshNames() {
        val asked = generation
        val read = bridge.onFfi { readNames(bridge.core) }
        if (asked != generation || !bridge.connected) return
        val next = read.copy(renameStamps = _names.value.renameStamps)
        if (next != _names.value) _names.value = next
    }

    private fun readNames(core: DspiCoreApi): DeviceNames {
        var sourcesSupported = false
        val sources = List(DeviceNames.SOURCE_SLOTS) { id ->
            try {
                core.cachedInputSourceName(id).also { sourcesSupported = true }
            } catch (e: DspiException) {
                ""
            }
        }
        val device = try {
            core.cachedDeviceName()
        } catch (e: DspiException) {
            null
        }
        return DeviceNames(device != null, device.orEmpty(), sourcesSupported, sources)
    }

    override suspend fun setDeviceName(name: String): Boolean = writeName("setDeviceName") { core -> core.setDeviceName(name) }

    override suspend fun setSourceName(source: Int, name: String): Boolean =
        writeName("setInputSourceName") { core -> core.setInputSourceName(source, name) }

    private suspend fun writeName(command: String, write: (DspiCoreApi) -> Unit): Boolean {
        if (bridge.writeRefused(command)) return false
        val error = bridge.onFfi {
            try {
                write(bridge.core)
                null
            } catch (e: DspiException) {
                e
            }
        }
        if (error != null) bridge.emit(BridgeEvent.CommandFailed(command, error))
        refreshNames()
        return error == null
    }

    override fun renameSource(source: Int, name: String) {
        if (source !in 0 until DeviceNames.SOURCE_SLOTS) return
        val current = _names.value
        val stamps = List(DeviceNames.SOURCE_SLOTS) { current.stamp(it) + if (it == source) 1 else 0 }
        _names.value = current.copy(renameStamps = stamps)
        val clean = DeviceNames.clean(name).trim()
        if (clean == current.source(source)) return
        val asked = generation
        bridge.scope.launch(bridge.main) {
            // A changed device gets none of the old one's names
            if (asked == generation) setSourceName(source, clean)
        }
    }

    // ── Output latency ──

    private class LatencyStatus(val requested: Int, val pending: Boolean)

    private fun parse(reply: OutputLatency): LatencyStatus? {
        fun field(name: String) = reply.fields.firstOrNull { it.name == name }?.value?.toIntOrNull()
        val requested = field("requested_id") ?: return null
        val pending = field("pending") ?: return null
        return LatencyStatus(requested, pending != 0)
    }

    override suspend fun refreshLatency() {
        if (!bridge.connected) return
        val asked = generation
        val read = bridge.onFfi {
            try {
                val status = parse(bridge.get { bridge.core.fetchOutputLatency() })
                status?.let {
                    OutputLatencyState(true, it.requested, it.pending, bridge.get { bridge.core.fetchOutputLatencyOptions() })
                } ?: OutputLatencyState()
            } catch (e: DspiException.Unsupported) {
                OutputLatencyState()
            } catch (e: DspiException) {
                null // a failed read (the flash window) says nothing: keep what was read
            }
        }
        if (read != null && asked == generation && bridge.connected) _latency.value = read
    }

    override suspend fun setOutputLatency(id: Int): LatencyApply {
        val command = "setOutputLatency"
        if (bridge.writeRefused(command)) return LatencyApply.NOT_SENT
        val asked = generation
        val error = bridge.onFfi {
            try {
                bridge.core.setOutputLatency(id)
                null
            } catch (e: DspiException) {
                e
            }
        }
        if (error != null) {
            bridge.emit(BridgeEvent.CommandFailed(command, error))
            return LatencyApply.NOT_SENT
        }
        // A failed read is the flash window: keep waiting. Never sent again.
        var outcome = LatencyApply.NOT_YET_APPLIED
        var waited = 0L
        while (waited < LATENCY_SETTLE_MS && asked == generation) {
            delay(LATENCY_POLL_MS)
            waited += LATENCY_POLL_MS
            val status = bridge.onFfi {
                try {
                    parse(bridge.core.fetchOutputLatency())
                } catch (e: DspiException) {
                    null
                }
            } ?: continue
            if (!status.pending) {
                outcome = if (status.requested == id) LatencyApply.APPLIED else LatencyApply.KEPT
                break
            }
        }
        // The table's effective values follow the new option
        if (asked == generation) refreshLatency()
        return outcome
    }

    // ── Core voltage ──

    private suspend fun probeCoreVoltage() {
        val asked = generation
        val listed = bridge.onFfi {
            try {
                bridge.core.extStatus().hasCoreVreg
            } catch (e: DspiException) {
                false
            }
        }
        if (asked != generation || !bridge.connected) return
        _coreVoltage.value = CoreVoltageState(listed)
        if (listed) refreshCoreVoltage()
    }

    override suspend fun refreshCoreVoltage() {
        if (!_coreVoltage.value.supported) return
        val asked = generation
        val status = bridge.onFfi {
            try {
                bridge.get { bridge.core.fetchCoreVreg() }
            } catch (e: DspiException) {
                null
            }
        } ?: return
        if (asked == generation && _coreVoltage.value.supported) _coreVoltage.value = CoreVoltageState(true, status)
    }

    private companion object {
        // As the Mac's 40 x 100 ms (Commands.swift:757-762)
        const val LATENCY_POLL_MS = 100L
        const val LATENCY_SETTLE_MS = 4_000L
    }
}
