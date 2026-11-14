// -- WP P1AN2 --
// The mode switch (docs/UI-MODES.md sections 1, 3 and 5): Simple, Advanced
// and Installer from the core's manifest, remembered per device (by serial)
// with a console-wide choice, the locked-user state when the device carries a
// lock, and a mode the hub pins. The core decides (`mode_resolve`); this keeps
// the user's choice, asks for the PIN only when one is set, and never writes
// to the device.
package se.bofr.dspi.app

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.LockState
import se.bofr.dspi.core.ModeChoice
import se.bofr.dspi.core.ModeEntry
import se.bofr.dspi.ui.UiModes

/** Where the chosen mode is kept: per device, and the console-wide choice. */
interface UiModeStore {
    /** The mode chosen for [serial]: its own, else the console-wide one; "" none (the app default). */
    fun chosen(serial: String): String

    /** Keep [mode] for [serial], or for every device ([everyDevice]: the device's own choice is dropped). */
    fun save(serial: String, mode: String, everyDevice: Boolean)
}

/** For tests and previews. */
class InMemoryUiModeStore : UiModeStore {
    private val perDevice = HashMap<String, String>()
    private var all = ""

    override fun chosen(serial: String) = perDevice[serial] ?: all

    override fun save(serial: String, mode: String, everyDevice: Boolean) {
        if (everyDevice) {
            all = mode
            perDevice.remove(serial)
        } else {
            perDevice[serial] = mode
        }
    }
}

/** The app's store, in its own preferences file. */
class PrefsUiModeStore(context: Context) : UiModeStore {
    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    override fun chosen(serial: String): String = prefs.getString(KEY_DEVICE + serial, null) ?: prefs.getString(KEY_ALL, "").orEmpty()

    override fun save(serial: String, mode: String, everyDevice: Boolean) {
        val e = prefs.edit()
        if (everyDevice) {
            e.putString(KEY_ALL, mode).remove(KEY_DEVICE + serial)
        } else {
            e.putString(KEY_DEVICE + serial, mode)
        }
        e.apply()
    }

    private companion object {
        const val PREFS = "dspi_ui_modes"
        const val KEY_ALL = "all"
        const val KEY_DEVICE = "device."
    }
}

/** The Mode dialog while open: what it offers, and its state. */
data class ModeDialogState(
    /** The modes the switch offers, in the manifest's order. */
    val modes: List<ModeEntry> = emptyList(),
    /** The mode ticked (the current one when opened). */
    val selected: String = "",
    /** Keep it for every device on this console, not only this one. */
    val everyDevice: Boolean = false,
    /** The locked-user state: the dialog is the Installer unlock. */
    val unlockOnly: Boolean = false,
    /** The scope line (`installer_lock.scope` or `installer_lock.hub`), "" none. */
    val scope: String = "",
    val busy: Boolean = false,
    /** The last refusal, in the core's words ("" none). */
    val error: String = ""
)

/**
 * The resolved mode of [bridge]'s device (main thread). [mode] is what the
 * screens are shown in; [choice] the core's whole answer (null until read, or
 * with a core that has no manifest: then Advanced, today's screens).
 */
class UiModeModel(private val bridge: DspiBridge, private val store: UiModeStore, parent: CoroutineScope) {
    /** Its own child of [parent]: [close] ends it (a new bridge gets a new model). */
    private val scope = CoroutineScope(parent.coroutineContext + SupervisorJob(parent.coroutineContext[Job]))

    fun close() = scope.cancel()

    private val installer get() = bridge.installer

    private val _choice = MutableStateFlow<ModeChoice?>(null)
    val choice: StateFlow<ModeChoice?> = _choice.asStateFlow()

    private val _mode = MutableStateFlow(UiModes.ADVANCED)
    val mode: StateFlow<String> = _mode.asStateFlow()

    private val _dialog = MutableStateFlow<ModeDialogState?>(null)
    val dialog: StateFlow<ModeDialogState?> = _dialog.asStateFlow()

    val lock: StateFlow<LockState?> get() = installer.lock

    init {
        scope.launch {
            combine(bridge.deviceGeneration, installer.lock) { g, l -> g to l }.collect { refresh() }
        }
    }

    /** Resolve again (a device, a lock or a choice changed). */
    suspend fun refresh() {
        val c = installer.resolve(store.chosen(bridge.selectedSerial))
        _choice.value = c
        _mode.value = c?.mode?.ifEmpty { null } ?: UiModes.ADVANCED
    }

    /** The current mode's label ("Simple"), "" before the first read. */
    suspend fun label(): String = installer.manifest()?.let { m -> modeLabel(m.modes, _mode.value, m.lockedUser.label) }.orEmpty()

    /** Open the Mode dialog (the Installer unlock while locked). Nothing when the hub pins the mode. */
    fun openDialog() {
        val c = _choice.value
        scope.launch {
            val m = installer.manifest() ?: return@launch
            val locked = c?.mode == UiModes.LOCKED_USER
            if (!locked && c?.switchable == false) return@launch
            val l = installer.lock.value
            val scopeLine = l?.scopeNoticeKey?.takeIf { l.pinSet && it.isNotEmpty() }?.let { installer.notice(it) }?.reason.orEmpty()
            _dialog.value =
                ModeDialogState(
                    modes = m.modes,
                    selected = if (locked) UiModes.INSTALLER else _mode.value,
                    unlockOnly = locked,
                    scope = scopeLine
                )
        }
    }

    fun select(mode: String) {
        _dialog.value = _dialog.value?.copy(selected = mode, error = "")
    }

    fun setEveryDevice(on: Boolean) {
        _dialog.value = _dialog.value?.copy(everyDevice = on)
    }

    fun closeDialog() {
        _dialog.value = null
    }

    /**
     * Whether the dialog asks for the PIN: only when one is set and this
     * console is not unlocked, for the Installer mode or the unlock (UI-MODES
     * section 1: with no PIN set nothing is ever asked).
     */
    fun needsPin(d: ModeDialogState): Boolean {
        val l = installer.lock.value ?: return false
        return l.pinSet && !l.unlocked && (d.unlockOnly || d.selected == UiModes.INSTALLER)
    }

    /** OK: unlock when the PIN is asked, then keep the choice and resolve again. Never writes to the device. */
    fun confirm(pin: String) {
        val d = _dialog.value ?: return
        _dialog.value = d.copy(busy = true, error = "")
        scope.launch {
            if (needsPin(d)) {
                val r = installer.unlock(pin)
                r.exceptionOrNull()?.let {
                    _dialog.value = _dialog.value?.copy(busy = false, error = it.message.orEmpty())
                    return@launch
                }
            }
            if (!d.unlockOnly) store.save(bridge.selectedSerial, d.selected, d.everyDevice)
            refresh()
            _dialog.value = null
        }
    }

    /** "Lock now": the installer leaves, the device is locked again on this console (or the hub). */
    fun lockNow() {
        scope.launch {
            installer.lockNow()
            refresh()
        }
    }

    companion object {
        /** [id]'s label in [modes] ([lockedLabel] for the locked-user state). */
        fun modeLabel(modes: List<ModeEntry>, id: String, lockedLabel: String = "Locked"): String =
            if (id == UiModes.LOCKED_USER) lockedLabel else modes.firstOrNull { it.id == id }?.label ?: id

        /** The routes the locked-user state shows (UI-MODES section 2: Home only; Settings shows About and the installer's note). */
        fun allowedWhenLocked(route: String): Boolean = route.substringBefore('/').substringBefore('?') in LOCKED_ROUTES

        private val LOCKED_ROUTES = setOf(
            se.bofr.dspi.ui.DspiRoutes.HOME,
            se.bofr.dspi.ui.DspiRoutes.SETTINGS,
            se.bofr.dspi.ui.DspiRoutes.GETTING_STARTED,
            se.bofr.dspi.ui.DspiRoutes.WHATS_NEW
        )
    }
}
