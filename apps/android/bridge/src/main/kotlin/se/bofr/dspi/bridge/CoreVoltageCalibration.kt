// WP AN23. CG11's calibration flow and CG14's manual ops for Advanced's Core
// Voltage page. The core holds the flow and every rule; this only takes its
// calls off the main thread and publishes its view.
package se.bofr.dspi.bridge

import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import se.bofr.dspi.core.DspiCoreApi
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.VregCalView

/**
 * The core-voltage calibration flow (CG11) and the manual ops (CG14) on the
 * connected device. Range, soak, the `OVERVOLT` word and "never a `SET` by
 * itself" are the core's; the view says what applies. The host keeps no
 * authority: a page that comes back, or a new process, finds a running trial
 * by reading the device again. Main thread.
 */
interface CoreVoltageCalibration {
    /** The core's view as of the last read; null before the page first showed on this device. */
    val view: StateFlow<VregCalView?>

    /**
     * The page is on screen (true) or not. While it is, the flow is open and
     * the device is read about once a second. Hiding it closes the flow,
     * which the core refuses during a run: the run goes on until ended.
     */
    fun setShown(shown: Boolean)

    /** Each returns null once the call went out, or the core's refusal; nothing is retried. */
    suspend fun start(target: Int, soakS: Long, typed: String?): DspiException?

    suspend fun keep(soakS: Long): DspiException?

    suspend fun commit(typed: String?): DspiException?

    suspend fun endTrial(): DspiException?

    /** `value` 0 is the reset to the build default. */
    suspend fun manualSet(value: Int, typed: String?): DspiException?

    suspend fun manualTrial(value: Int, typed: String?): DspiException?

    suspend fun manualAuto(on: Boolean): DspiException?

    suspend fun manualClear(): DspiException?
}

/** See [CoreVoltageCalibration]. */
class CoreVoltageCalibrationImpl internal constructor(private val bridge: DspiBridgeImpl) : CoreVoltageCalibration {
    private val _view = MutableStateFlow<VregCalView?>(null)
    override val view: StateFlow<VregCalView?> = _view.asStateFlow()

    private var shown = false
    private var pollJob: Job? = null
    private var seenGeneration = 0L

    init {
        bridge.deviceChangedHooks += {
            if (seenGeneration != generation) {
                seenGeneration = generation
                _view.value = null
                // A new connection: open the flow on it, which reads trial_active
                if (shown) restart()
            }
        }
    }

    private val generation: Long get() = bridge.deviceGeneration.value

    override fun setShown(shown: Boolean) {
        if (this.shown == shown) return
        this.shown = shown
        if (shown) {
            restart()
        } else {
            pollJob?.cancel()
            pollJob = null
            bridge.scope.launch(bridge.main) {
                // Refused during a run, which goes on; nothing to show for it
                bridge.onFfi { attempt { it.coreVregCalClose() } }
                // Overview's status rows read the device again
                bridge.deviceSettings.refreshCoreVoltage()
            }
        }
    }

    private fun restart() {
        pollJob?.cancel()
        pollJob = bridge.scope.launch(bridge.main) {
            // Disconnected is refused with no traffic; the poll's view then shows a run's phase with the device away
            bridge.onFfi { attempt { it.coreVregCalOpen() } }
            while (isActive) {
                poll()
                delay(POLL_MS)
            }
        }
    }

    private suspend fun poll() {
        val asked = generation
        val next = bridge.onFfi { bridge.core.coreVregCalPoll() }
        if (asked == generation) _view.value = next
    }

    override suspend fun start(target: Int, soakS: Long, typed: String?) = act { it.coreVregCalStart(target, soakS, typed) }

    override suspend fun keep(soakS: Long) = act { it.coreVregCalKeep(soakS) }

    override suspend fun commit(typed: String?) = act { it.coreVregCalCommit(typed) }

    override suspend fun endTrial() = act { it.coreVregTrialEnd() }

    override suspend fun manualSet(value: Int, typed: String?) = act { it.coreVregManualSet(value, typed) }

    override suspend fun manualTrial(value: Int, typed: String?) = act { it.coreVregManualTrial(value, typed) }

    override suspend fun manualAuto(on: Boolean) = act { it.coreVregManualAuto(on) }

    override suspend fun manualClear() = act { it.coreVregManualClear() }

    /** One call, then a read so the page shows what it did. */
    private suspend fun act(call: (DspiCoreApi) -> Unit): DspiException? {
        val error = bridge.onFfi { attempt(call) }
        poll()
        return error
    }

    private fun attempt(call: (DspiCoreApi) -> Unit): DspiException? = try {
        call(bridge.core)
        null
    } catch (e: DspiException) {
        e
    }

    internal fun close() {
        pollJob?.cancel()
        pollJob = null
    }

    private companion object {
        // The core: "about once a second while the page shows"
        const val POLL_MS = 1_000L
    }
}
