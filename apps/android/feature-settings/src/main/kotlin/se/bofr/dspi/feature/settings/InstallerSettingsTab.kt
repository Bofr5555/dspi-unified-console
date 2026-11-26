// -- WP P1AN2 --
// Settings > Installer (the Installer mode, docs/UI-MODES.md sections 2 and 3):
// the installer's PIN (set, change, clear, the recovery code shown once), what
// the lock covers and Lock now, the limits profile (P1LM) with its clamp
// report, and S2's protect all outputs. Changes apply at once (no Save bar):
// each is a write, journalled like any other. In the locked-user state the
// page shows the installer's note and contact only.
// -- WP HUBADMIN -- through a hub that keeps limits the page edits the HUB's
// copy (limits, PIN, what the lock covers, the roles' pinned modes), says so
// with the core's notice row, and is read-only below Admin.
package se.bofr.dspi.feature.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.bridge.Installer
import se.bofr.dspi.bridge.LimitsDraft
import se.bofr.dspi.bridge.OutputLimits
import se.bofr.dspi.core.HubAdminState
import se.bofr.dspi.core.InstallerTarget
import se.bofr.dspi.core.LimitFieldRange
import se.bofr.dspi.core.LimitsStatus
import se.bofr.dspi.core.LockState
import se.bofr.dspi.core.ModeScreen
import se.bofr.dspi.core.Notice
import se.bofr.dspi.ui.alert.LocalNoticeLine
import se.bofr.dspi.ui.onMainThread
import se.bofr.dspi.ui.writeEnabled

/** Test tags. */
const val PIN_FIELD_TAG = "installer-pin"
const val PIN_REPEAT_TAG = "installer-pin-repeat"
const val SET_PIN_TAG = "installer-set-pin"
const val RECOVERY_TAG = "installer-recovery"
const val LIMIT_REPORT_TAG = "installer-limit-report"

// -- WP HUBADMIN --
const val COPY_NOTICE_TAG = "installer-copy"
const val READ_ONLY_NOTICE_TAG = "installer-read-only"
const val PINNED_MODES_TAG = "installer-pinned-modes"

/** The Installer page's logic, apart from Compose (tests drive it directly). Main thread. */
class InstallerPageModel(private val bridge: DspiBridge, private val scope: CoroutineScope) {
    private val installer get() = bridge.installer

    /** The last outcome, one line ("" none); [isError] whether it failed. */
    data class Outcome(val text: String = "", val isError: Boolean = false)

    private val _outcome = MutableStateFlow(Outcome())
    val outcome: StateFlow<Outcome> = _outcome.asStateFlow()

    /** The recovery code, shown once after a PIN was set ("" none). */
    private val _recovery = MutableStateFlow("")
    val recovery: StateFlow<String> = _recovery.asStateFlow()

    private val _screens = MutableStateFlow<List<ModeScreen>>(emptyList())

    /** The manifest's screens, the lock's choices (`lockedPages`). */
    val screens: StateFlow<List<ModeScreen>> = _screens.asStateFlow()

    private val _busy = MutableStateFlow(false)
    val busy: StateFlow<Boolean> = _busy.asStateFlow()

    /** The words of `installer_lock.recovery` and `limits.scope` (the core's), loaded once. */
    var recoveryWords = ""
        private set
    var limitsScopeWords = ""
        private set

    // -- WP P1EFIX -- the editor's ranges: the core's one table (review p1e-a F7)
    private val _ranges = MutableStateFlow<Map<String, LimitFieldRange>>(emptyMap())

    /** Every limits field's range by field key (`maxGainDb`, ...), from the core; empty until read. */
    val ranges: StateFlow<Map<String, LimitFieldRange>> = _ranges.asStateFlow()

    /** The S2 warning: outputs above -10 dB gain without a limiter. */
    private val _unprotected = MutableStateFlow<List<Int>>(emptyList())
    val unprotected: StateFlow<List<Int>> = _unprotected.asStateFlow()

    // -- WP HUBADMIN --
    /** Which copy the page edits (the core's), and the hub's copy while it is the hub's. */
    val target: StateFlow<InstallerTarget?> get() = installer.target
    val hub: StateFlow<HubAdminState?> get() = installer.hub

    private val _copyLine = MutableStateFlow("")

    /** The notice row `noticeKey` in words: which copy is edited ("" before it is read). */
    val copyLine: StateFlow<String> = _copyLine.asStateFlow()

    private val _readOnlyLine = MutableStateFlow("")

    /** The notice row `readOnlyKey` in words: why the page is read-only ("" editable). */
    val readOnlyLine: StateFlow<String> = _readOnlyLine.asStateFlow()

    private val _recoveryNote = MutableStateFlow("")

    /** The words shown with the recovery code: `installer_lock.recovery`, or `installer_lock.hub_recovery` for the hub's PIN. */
    val recoveryNote: StateFlow<String> = _recoveryNote.asStateFlow()

    /** The words of `installer_lock.hub_recovery`, loaded once. */
    var hubRecoveryWords = ""
        private set

    /** The page may change the copy it shows (always this console's; the hub's as an Admin). */
    val editable: Boolean get() = !readOnly(target.value)

    private var watching = false

    fun load() {
        // -- WP HUBADMIN -- the copy's notice rows follow the target
        if (!watching) {
            watching = true
            scope.launch {
                installer.target.collect { t ->
                    _copyLine.value = t?.noticeKey?.takeIf { it.isNotEmpty() }?.let { installer.notice(it) }?.let(::noticeLine).orEmpty()
                    _readOnlyLine.value =
                        t?.takeIf { readOnly(it) }?.readOnlyKey?.takeIf { it.isNotEmpty() }?.let { installer.notice(it) }
                            ?.let { noticeLine(it, action = true) }.orEmpty()
                }
            }
        }
        scope.launch {
            installer.refresh(hub = true)
            _screens.value = installer.manifest()?.screens.orEmpty()
            recoveryWords = installer.notice(RECOVERY_KEY)?.let { "${it.reason} ${it.action}" }.orEmpty()
            limitsScopeWords = installer.notice(LIMITS_SCOPE_KEY)?.reason.orEmpty()
            hubRecoveryWords = installer.notice(HUB_RECOVERY_KEY)?.let { "${it.reason} ${it.action}" }.orEmpty() // -- WP HUBADMIN --
            _ranges.value = installer.fieldRanges() // -- WP P1EFIX --
            _unprotected.value = installer.outputsWithoutLimiter()
        }
    }

    private fun run(block: suspend () -> Outcome?) {
        _busy.value = true
        scope.launch {
            try {
                block()?.let { _outcome.value = it }
            } finally {
                _busy.value = false
            }
        }
    }

    private fun failed(e: Throwable?) = Outcome(e?.message.orEmpty().ifEmpty { "That did not work." }, isError = true)

    // -- WP HUBADMIN -- read-only (below Admin on the hub, a hub without limits): nothing is sent, the row says why
    private fun guarded(block: suspend () -> Outcome?) {
        if (!editable) {
            _outcome.value = Outcome(_readOnlyLine.value.ifEmpty { "Read-only." }, isError = true)
            return
        }
        run(block)
    }

    /** Set (or change) the PIN: [pin] twice; then the recovery code is shown once. */
    fun setPin(pin: String, repeat: String) {
        if (pin != repeat) {
            _outcome.value = Outcome("The two PINs differ.", isError = true)
            return
        }
        val onHub = target.value?.copy == Installer.COPY_HUB // -- WP HUBADMIN --
        guarded {
            installer.setPin(pin).fold(
                onSuccess = { code ->
                    _recoveryNote.value = if (onHub) hubRecoveryWords else recoveryWords
                    _recovery.value = code
                    Outcome("Installer PIN set.")
                },
                onFailure = ::failed
            )
        }
    }

    fun recoveryNoted() {
        _recovery.value = ""
        _recoveryNote.value = ""
    }

    fun clearPin() = guarded { installer.clearPin().fold({ Outcome("Installer PIN removed: nothing is locked.") }, ::failed) }

    fun lockNow() = run { installer.lockNow().fold({ Outcome("Locked.") }, ::failed) }

    /** What the lock covers; applied at once. */
    fun setCovered(page: String, on: Boolean) {
        val now = covered() // -- WP HUBADMIN --
        val pages = if (on) (now + page).distinct() else now - page
        guarded { installer.cover(pages).fold({ null }, ::failed) }
    }

    /** Apply [draft] at once: the clamp report is the outcome (S1, F3). */
    fun apply(draft: LimitsDraft) = guarded {
        installer.applyLimits(draft).fold(
            onSuccess = { r ->
                _unprotected.value = installer.outputsWithoutLimiter()
                Outcome(r.summary)
            },
            onFailure = ::failed
        )
    }

    fun clearLimits() = guarded { installer.clearLimits().fold({ Outcome("Limits removed.") }, ::failed) }

    // -- WP HUBADMIN --
    /** What the lock covers: on the hub the profile's `lockedPages`, here the lock's. */
    fun covered(): List<String> = if (target.value?.copy == Installer.COPY_HUB) {
        installer.profile.value?.lockedPages.orEmpty()
    } else {
        installer.lock.value?.covers.orEmpty()
    }

    /** Pin [role]'s mode on the hub ("" none). */
    fun pinMode(role: String, mode: String) = guarded {
        installer.pinMode(role, mode).fold({ Outcome("${roleTitle(role)}: ${modeTitle(mode)}.") }, ::failed)
    }

    fun protectAll() = run {
        installer.protectAllOutputs().fold(
            onSuccess = {
                _unprotected.value = installer.outputsWithoutLimiter()
                Outcome("Every output's limiter is on at ${Installer.PROTECT_CEILING_DB.toInt()} dBFS, pairs linked.")
            },
            onFailure = ::failed
        )
    }

    companion object {
        const val RECOVERY_KEY = "installer_lock.recovery"
        const val LIMITS_SCOPE_KEY = "limits.scope"

        // -- WP HUBADMIN --
        const val HUB_RECOVERY_KEY = "installer_lock.hub_recovery"

        /** Read-only: the core says this console may not change the copy. Null (not read yet, no core): as before. */
        fun readOnly(t: InstallerTarget?): Boolean = t != null && !t.editable

        /** A notice row in one line: "Title: reason" (and its action). */
        fun noticeLine(n: Notice, action: Boolean = false): String =
            listOf("${n.title}: ${n.reason}", if (action) n.action else "").filter { it.isNotBlank() }.joinToString(" ")

        fun roleTitle(role: String): String = when (role) {
            "viewer" -> "Viewer"
            "control" -> "Control"
            "admin" -> "Admin"
            else -> role
        }

        fun modeTitle(mode: String): String = when (mode) {
            "" -> "None"
            Installer.SIMPLE -> "Simple"
            Installer.ADVANCED -> "Advanced"
            Installer.INSTALLER -> "Installer"
            else -> mode
        }

        /** The hub's limits in one line. */
        fun hubLimitsLine(profile: LimitsDraft?): String =
            if (profile == null) "No limits profile on the hub: nothing is limited." else "The hub's limits profile is in force."

        /** The lock's state in one line. */
        fun lockLine(l: LockState?): String = when {
            l == null -> "No device."
            !l.pinSet -> "No installer PIN is set: nothing is locked and nothing asks for a PIN."
            l.heldBy == Installer.HELD_BY_HUB && l.unlocked -> "The hub holds this DSPi's lock. Unlocked for this session."
            l.heldBy == Installer.HELD_BY_HUB -> "The hub holds this DSPi's lock."
            l.recovered -> "Unlocked with the recovery code. Set a new PIN."
            l.unlocked -> "Installer PIN set. Unlocked on this console for this session."
            l.covers.isEmpty() -> "Installer PIN set. The lock covers nothing yet."
            else -> "Installer PIN set and locked."
        }

        /** The limits' state in one line ("" none). */
        fun limitsLine(s: LimitsStatus?, words: String): String = when (s?.state) {
            null, "none" -> "No limits profile: nothing is limited."
            "ok" -> "A limits profile is in force."
            else -> words.ifEmpty { s.detail }
        }

        // -- WP P1EFIX --
        /** [v] kept within the core's [range] (null: as typed; the core clamps all the same). */
        fun inRange(v: Float?, range: LimitFieldRange?): Float? = if (v == null || range == null) v else v.coerceIn(range.min, range.max)

        /** The field's range in words: "-60 to 20 dB". */
        fun rangeLine(r: LimitFieldRange): String = "${formatLimit(r.min)} to ${formatLimit(r.max)} ${r.unit}"

        /** "OUT3, OUT4 have no limiter…" for [outs] (0-based), "" none. */
        fun unprotectedLine(outs: List<Int>): String = if (outs.isEmpty()) {
            ""
        } else {
            outs.joinToString(", ") { "OUT${it + 1}" } +
                (if (outs.size == 1) " is" else " are") + " above ${Installer.UNPROTECTED_GAIN_DB.toInt()} dB gain with no limiter."
        }
    }
}

@Composable
internal fun InstallerSettingsTab(bridge: DspiBridge, lockedUser: Boolean) {
    val scope = rememberCoroutineScope()
    val model = remember(bridge) { InstallerPageModel(bridge, scope) }
    LaunchedEffect(model) { model.load() }
    val lock by bridge.installer.lock.collectAsState()
    val limits by bridge.installer.limits.collectAsState()
    val profile by bridge.installer.profile.collectAsState()
    val outcome by model.outcome.collectAsState()
    val recovery by model.recovery.collectAsState()
    val screens by model.screens.collectAsState()
    val busy by model.busy.collectAsState()
    val unprotected by model.unprotected.collectAsState()
    // -- WP HUBADMIN --
    val target by model.target.collectAsState()
    val hub by model.hub.collectAsState()
    val copyLine by model.copyLine.collectAsState()
    val readOnlyLine by model.readOnlyLine.collectAsState()
    val recoveryNote by model.recoveryNote.collectAsState()
    val onHub = target?.copy == Installer.COPY_HUB
    val editable = !InstallerPageModel.readOnly(target)
    val state by bridge.state.collectAsState()
    val connected = bridge.status.collectAsState().value.connected
    val draft = profile ?: LimitsDraft()
    val noticeLine = LocalNoticeLine.current
    LaunchedEffect(outcome) { if (outcome.text.isNotEmpty()) noticeLine?.invoke(outcome.text) }
    // The one line the unlock dialog shows too: the lock is on this console (or the hub), the DSPi itself is open
    var scopeLine by remember { mutableStateOf("") }
    LaunchedEffect(lock?.scopeNoticeKey, lock?.pinSet) {
        val line =
            lock?.takeIf { it.pinSet }?.scopeNoticeKey?.takeIf { it.isNotEmpty() }?.let { bridge.installer.notice(it)?.reason }.orEmpty()
        onMainThread { scopeLine = line } // -- WP FLAKE2 -- the write on the main thread (onMainThread)
    }

    SettingsForm {
        if (lockedUser) {
            item {
                SettingsSection("Installer") {
                    // -- FIX016 -- (LOCKSRC) the source passes the lock too
                    FormCaption("This DSPi is locked by the installer: volume, mute, source and presets can be changed.")
                    lock?.note?.takeIf { it.isNotBlank() }?.let { FormCaption(it) }
                    lock?.contact?.takeIf { it.isNotBlank() }?.let { FormCaption("Contact: $it") }
                    FormCaption("To unlock, choose Unlock… in the top bar and enter the installer's PIN.")
                }
            }
            return@SettingsForm
        }
        // The last change's outcome (the clamp report of a limit), at the top where it is seen; the notice line says it too
        item {
            Column(Modifier.testTag(LIMIT_REPORT_TAG)) { StatusLine(outcome.text, outcome.isError) }
        }
        // -- WP HUBADMIN -- which copy is edited, and why it is read-only (the core's rows)
        if (copyLine.isNotEmpty() || readOnlyLine.isNotEmpty()) {
            item {
                Column(Modifier.fillMaxWidth()) {
                    if (copyLine.isNotEmpty()) Column(Modifier.testTag(COPY_NOTICE_TAG)) { FormCaption(copyLine) }
                    if (readOnlyLine.isNotEmpty()) {
                        Column(Modifier.testTag(READ_ONLY_NOTICE_TAG)) { StatusLine(readOnlyLine, isError = true) }
                    }
                }
            }
        }
        item {
            SettingsSection("Installer PIN", footer = scopeLine.ifEmpty { null }) {
                FormCaption(InstallerPageModel.lockLine(lock))
                if (recovery.isNotEmpty()) {
                    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).testTag(RECOVERY_TAG)) {
                        Text("Recovery code", style = MaterialTheme.typography.titleSmall)
                        Text(recovery, style = MaterialTheme.typography.titleMedium, fontFamily = FontFamily.Monospace)
                        if (recoveryNote.isNotEmpty()) Text(recoveryNote, style = MaterialTheme.typography.bodySmall) // -- WP HUBADMIN --
                        OutlinedButton(onClick = model::recoveryNoted) { Text("I have noted it") }
                    }
                } else if (onHub && editable) { // -- WP HUBADMIN -- the hub's PIN: an Admin needs no current one
                    PinEntry(hub?.pinSet == true, enabled = connected && !busy) { pin, repeat -> model.setPin(pin, repeat) }
                } else if (!onHub && editable && lock?.heldBy != Installer.HELD_BY_HUB) {
                    PinEntry(lock?.pinSet == true, enabled = connected && !busy) { pin, repeat -> model.setPin(pin, repeat) }
                }
                if (onHub && editable && hub?.pinSet == true) { // -- WP HUBADMIN --
                    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)) {
                        OutlinedButton(onClick = model::clearPin, enabled = !busy) { Text("Remove PIN") }
                    }
                }
                if (!onHub && editable && lock?.pinSet == true && lock?.heldBy != Installer.HELD_BY_HUB) {
                    Row(
                        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        OutlinedButton(onClick = model::lockNow, enabled = !busy && lock?.unlocked == true) { Text("Lock now") }
                        OutlinedButton(onClick = model::clearPin, enabled = !busy) { Text("Remove PIN") }
                    }
                }
            }
        }
        val pinSet = if (onHub) hub?.pinSet == true else lock?.pinSet == true // -- WP HUBADMIN --
        if (pinSet && screens.isNotEmpty()) {
            item {
                SettingsSection("What the lock covers", footer = "With nothing ticked, the PIN guards the Installer pages only.") {
                    val covered = if (onHub) draft.lockedPages else lock?.covers.orEmpty()
                    screens.forEach { s ->
                        val on = s.id in covered
                        Row(
                            Modifier.fillMaxWidth().padding(horizontal = 8.dp),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Checkbox(
                                checked = on,
                                onCheckedChange = { model.setCovered(s.id, it) },
                                enabled = writeEnabled(!busy && editable)
                            )
                            Text(s.title)
                        }
                    }
                }
            }
        }
        item {
            SettingsSection("Limits", footer = model.limitsScopeWords.ifEmpty { null }) {
                FormCaption(if (onHub) InstallerPageModel.hubLimitsLine(profile) else InstallerPageModel.limitsLine(limits, ""))
                val ranges by model.ranges.collectAsState() // -- WP P1EFIX --
                LimitField("Master volume, at most (dB)", draft.masterVolumeMaxDb, ranges["masterVolumeMaxDb"], enabled = editable) {
                    model.apply(draft.copy(masterVolumeMaxDb = it))
                }
                LimitField("EQ boost, at most (dB)", draft.maxPeqBoostDb, ranges["maxPeqBoostDb"], enabled = editable) {
                    model.apply(draft.copy(maxPeqBoostDb = it))
                }
                var out by remember { mutableStateOf(0) }
                val n = state.numOutputChannels.coerceAtLeast(1)
                ChoiceRow("Output", null, (0 until n).map { it to "OUT${it + 1}" }, out.coerceIn(0, n - 1)) { out = it }
                val o = draft.output(out)
                fun put(row: OutputLimits) = model.apply(draft.withOutput(row))
                LimitField("Gain, at most (dB)", o.maxGainDb, ranges["maxGainDb"], key = "g$out", enabled = editable) {
                    put(o.copy(maxGainDb = it))
                }
                LimitField("Limiter ceiling (dBFS)", o.limiterCeilingDb, ranges["limiterCeilingDb"], "l$out", editable) {
                    put(o.copy(limiterCeilingDb = it))
                }
                LimitField("Delay, at least (ms)", o.delayMinMs, ranges["delayMinMs"], key = "d$out", enabled = editable) {
                    put(o.copy(delayMinMs = it))
                }
                LimitField("Delay, at most (ms)", o.delayMaxMs, ranges["delayMaxMs"], key = "D$out", enabled = editable) {
                    put(o.copy(delayMaxMs = it))
                }
                LimitField("High-pass, at least (Hz)", o.highpassMinHz, ranges["highpassMinHz"], "h$out", editable) {
                    put(o.copy(highpassMinHz = it))
                }
                LimitField(
                    "High-pass slope, at least (dB/oct)",
                    o.highpassMinSlopeDb?.toFloat(),
                    ranges["highpassMinSlopeDb"],
                    key = "s$out",
                    enabled = editable
                ) {
                    put(o.copy(highpassMinSlopeDb = it?.toInt()))
                }
                TextLimitField("Installer's note", draft.note, enabled = editable) { model.apply(draft.copy(note = it)) }
                TextLimitField("Contact", draft.contact, enabled = editable) { model.apply(draft.copy(contact = it)) }
                if (profile != null) {
                    Row(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
                        OutlinedButton(onClick = model::clearLimits, enabled = !busy && editable) { Text("Remove limits") }
                    }
                }
            }
        }
        // -- WP HUBADMIN -- the roles' pinned modes on the hub (an Admin sees all three)
        if (onHub && hub?.pinnedModes?.isNotEmpty() == true) {
            item {
                SettingsSection(
                    "Pinned modes",
                    Modifier.testTag(PINNED_MODES_TAG),
                    footer = "A pinned mode is the only mode that role's consoles open."
                ) {
                    val pins = hub?.pinnedModes.orEmpty()
                    Installer.HUB_ROLES.mapNotNull { r -> pins.firstOrNull { it.role == r } }.forEach { p ->
                        ChoiceRow(
                            InstallerPageModel.roleTitle(p.role),
                            null,
                            Installer.PINNABLE_MODES.map { it to InstallerPageModel.modeTitle(it) },
                            p.mode,
                            enabled = connected && !busy && editable
                        ) { model.pinMode(p.role, it) }
                    }
                }
            }
        }
        item {
            SettingsSection("Protect all outputs") {
                val warn = InstallerPageModel.unprotectedLine(unprotected)
                if (warn.isNotEmpty()) StatusLine(warn, isError = true)
                SettingRow(
                    "Protect all outputs",
                    "Every output's limiter on at ${Installer.PROTECT_CEILING_DB.toInt()} dBFS, stereo pairs linked. One undo step."
                ) {
                    OutlinedButton(onClick = model::protectAll, enabled = writeEnabled(connected && !busy)) { Text("Protect") }
                }
            }
        }
    }
}

/** The PIN twice and Set PIN (Change PIN when one is set). */
@Composable
private fun PinEntry(pinSet: Boolean, enabled: Boolean, onSet: (String, String) -> Unit) {
    var pin by remember { mutableStateOf("") }
    var repeat by remember { mutableStateOf("") }
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        OutlinedTextField(
            pin,
            { pin = it },
            label = { Text(if (pinSet) "New PIN" else "PIN (4 to 64 characters)") },
            singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            modifier = Modifier.fillMaxWidth().testTag(PIN_FIELD_TAG)
        )
        OutlinedTextField(
            repeat,
            { repeat = it },
            label = { Text("The same PIN again") },
            singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            modifier = Modifier.fillMaxWidth().testTag(PIN_REPEAT_TAG)
        )
        OutlinedButton(
            onClick = {
                onSet(pin, repeat)
                pin = ""
                repeat = ""
            },
            enabled = enabled && pin.isNotEmpty() && repeat.isNotEmpty(),
            modifier = Modifier.testTag(SET_PIN_TAG)
        ) { Text(if (pinSet) "Change PIN" else "Set PIN") }
    }
}

/** A number limit: empty limits nothing. Applied on Done or when the field loses focus, if it changed. */
@Composable
private fun LimitField(
    label: String,
    value: Float?,
    range: LimitFieldRange?,
    key: String = label,
    enabled: Boolean = true, // -- WP HUBADMIN -- read-only below Admin on the hub
    onCommit: (Float?) -> Unit
) {
    val shown = value?.let { formatLimit(it) }.orEmpty()
    var text by remember(key, shown) { mutableStateOf(shown) }
    fun commit() {
        if (text.trim() == shown) return
        val v = text.trim().replace(',', '.').ifEmpty { null }?.toFloatOrNull()
        if (text.isNotBlank() && v == null) {
            text = shown
            return
        }
        // -- WP P1EFIX -- kept in the core's range, as every client's editor (the core clamps anyway)
        onCommit(InstallerPageModel.inRange(v, range))
    }
    OutlinedTextField(
        text,
        { text = it },
        label = { Text(label) },
        supportingText = range?.let { r -> { Text(InstallerPageModel.rangeLine(r)) } },
        singleLine = true,
        enabled = writeEnabled(enabled),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal, imeAction = ImeAction.Done),
        keyboardActions = KeyboardActions(onDone = { commit() }),
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 2.dp)
            .onFocusChanged { if (!it.isFocused) commit() }
            .testTag("limit-$label")
    )
}

/** A text limit (note, contact): applied on Done or focus loss. */
@Composable
private fun TextLimitField(label: String, value: String, enabled: Boolean = true, onCommit: (String) -> Unit) {
    var text by remember(value) { mutableStateOf(value) }
    fun commit() {
        if (text != value) onCommit(text.trim())
    }
    OutlinedTextField(
        text,
        { text = it },
        label = { Text(label) },
        singleLine = true,
        enabled = writeEnabled(enabled),
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
        keyboardActions = KeyboardActions(onDone = { commit() }),
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 2.dp).onFocusChanged { if (!it.isFocused) commit() }
    )
}

/** "-10", "2.5". */
internal fun formatLimit(v: Float): String = if (v == v.toInt().toFloat()) v.toInt().toString() else v.toString()
