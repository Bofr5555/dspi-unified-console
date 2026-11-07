// WP AN23. Core-voltage calibration (CG11) and the manual ops (CG14), reached
// only from Advanced. A thin page over the core's flow: the warning, the
// result, the voltages offered and what needs the word are the core's view;
// this page only picks and sends.
package se.bofr.dspi.feature.settings

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import java.util.Locale
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.VregCalPhase
import se.bofr.dspi.core.VregCalView
import se.bofr.dspi.core.VregChoice
import se.bofr.dspi.ui.MacColors
import se.bofr.dspi.ui.WhileVisible

internal const val CORE_VOLTAGE_TITLE = "Core Voltage"
internal const val CORE_VOLTAGE_MESSAGE_TAG = "coreVoltage.message"
internal const val CORE_VOLTAGE_WORD_TAG = "coreVoltage.word"
internal const val CORE_VOLTAGE_ERROR_TAG = "coreVoltage.error"

/** The phases a run is under way in: only an abort applies. */
private val RUNNING =
    setOf(VregCalPhase.CLEARING, VregCalPhase.STARTING_TRIAL, VregCalPhase.SOAKING, VregCalPhase.ENDING, VregCalPhase.COMMITTING)

/** Soaks to choose from, minutes; the core's minimum and default are always among them. */
private val SOAK_MINUTES = listOf(10L, 30L, 60L, 120L)

@Composable
fun CoreVoltageScreen(bridge: DspiBridge, canWrite: Boolean, onBack: () -> Unit) {
    val cal = bridge.coreVoltageCalibration
    val view by cal.view.collectAsState()
    val scope = rememberCoroutineScope()
    var error by remember { mutableStateOf<String?>(null) }

    BackHandler(onBack = onBack)
    // Polls only while on screen; the soak counts on the device's clock, so a page that was away reads the result
    WhileVisible(cal, onShow = { cal.setShown(true) }, onHide = { cal.setShown(false) })

    val send: (suspend () -> DspiException?) -> Unit = { call ->
        scope.launch { error = call()?.let(::refusalText) }
    }
    val platform = bridge.platformName

    SettingsForm {
        item {
            Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp)) { TextButton(onClick = onBack) { Text("‹ Advanced") } }
        }
        val v = view
        if (v == null) {
            item { FormCaption("Reading…") }
            return@SettingsForm
        }
        if (!v.available && v.phase == VregCalPhase.IDLE) {
            item { FormCaption(v.lastError ?: "Not available on this connection.") }
            return@SettingsForm
        }
        item { SettingsSection(CORE_VOLTAGE_TITLE) { FormCaption(v.warning, MacColors.orange) } }
        item {
            SettingsSection("Calibration") {
                Text(
                    v.message,
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp).testTag(CORE_VOLTAGE_MESSAGE_TAG)
                )
                if (v.phase == VregCalPhase.SOAKING || v.phase == VregCalPhase.PASSED) {
                    SettingRow("Soak") { Text("${formatMinutes(v.elapsedS)} of ${formatMinutes(v.soakS)}") }
                }
                if (v.linkLost) FormCaption("No answer from the device since the last reading.", MacColors.orange)
                CalibrationActions(v, platform, canWrite && v.available, send, cal)
                error?.let {
                    Text(
                        it,
                        color = MacColors.orange,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp).testTag(CORE_VOLTAGE_ERROR_TAG)
                    )
                }
            }
        }
        v.status?.let { status ->
            item {
                SettingsSection("On the device") {
                    coreVoltageRows(status, platform).forEach { (title, value) ->
                        SettingRow(title) { Text(value, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                    }
                }
            }
        }
        if (v.phase !in RUNNING && v.phase != VregCalPhase.PASSED) {
            item { ManualSection(v, platform, canWrite && v.available, send, cal) }
        }
    }
}

@Composable
private fun CalibrationActions(
    v: VregCalView,
    platform: String,
    enabled: Boolean,
    send: (suspend () -> DspiException?) -> Unit,
    cal: se.bofr.dspi.bridge.CoreVoltageCalibration
) {
    var soakS by rememberSaveable(v.defaultSoakS) { mutableStateOf(v.defaultSoakS) }
    val soaks = (SOAK_MINUTES.map { it * 60 } + v.minSoakS + v.defaultSoakS).filter { it >= v.minSoakS }.distinct().sorted()
    val soakOptions = soaks.map { it to formatMinutes(it) }

    when {
        v.canStart && v.choices.isNotEmpty() -> {
            // After a failure, the next step up; otherwise the lowest the flow offers
            var target by rememberSaveable(v.phase, v.nextVreg) {
                mutableStateOf(v.nextVreg.takeIf { n -> v.choices.any { it.code == n } } ?: v.choices.first().code)
            }
            val choice = v.choices.firstOrNull { it.code == target } ?: v.choices.first()
            var word by rememberSaveable(target) { mutableStateOf("") }
            ChoiceRow(
                "Voltage to try",
                null,
                v.choices.map {
                    it.code to vregLabel(it.code, platform)
                },
                choice.code,
                enabled
            ) { target = it }
            ChoiceRow("Soak", v.soakNote, soakOptions, soakS, enabled) { soakS = it }
            if (choice.needsWord) WordField(v.confirmationWord, word) { word = it }
            ActionRow {
                Button(onClick = {
                    send { cal.start(choice.code, soakS, word.ifEmpty { null }) }
                }, enabled = enabled) { Text("Start Trial") }
            }
        }
        v.canKeep -> {
            ChoiceRow("Soak", v.soakNote, soakOptions, soakS, enabled) { soakS = it }
            ActionRow {
                OutlinedButton(onClick = { send { cal.endTrial() } }, enabled = enabled) { Text("End Trial") }
                Button(onClick = { send { cal.keep(soakS) } }, enabled = enabled) { Text("Keep and Soak") }
            }
        }
        v.canCommit -> {
            var word by rememberSaveable(v.targetVreg) { mutableStateOf("") }
            if (v.commitNeedsWord) WordField(v.confirmationWord, word) { word = it }
            ActionRow {
                OutlinedButton(onClick = { send { cal.endTrial() } }, enabled = enabled) { Text("End Trial") }
                Button(onClick = { send { cal.commit(word.ifEmpty { null }) } }, enabled = enabled) {
                    Text("Save ${vregLabel(v.targetVreg, platform)}")
                }
            }
        }
        // Abort is open whenever a trial can be running, even while reads fail
        v.phase in RUNNING &&
            v.phase != VregCalPhase.COMMITTING &&
            v.phase != VregCalPhase.ENDING ||
            v.phase == VregCalPhase.TRIAL_LEFT_RUNNING ||
            v.phase == VregCalPhase.VERIFY_FAILED -> ActionRow {
            OutlinedButton(onClick = { send { cal.endTrial() } }, enabled = canEnd(enabled, v)) {
                Text(
                    if (v.phase == VregCalPhase.SOAKING ||
                        v.phase == VregCalPhase.CLEARING ||
                        v.phase == VregCalPhase.STARTING_TRIAL
                    ) {
                        "Abort"
                    } else {
                        "End Trial"
                    }
                )
            }
        }
    }
}

/** The way out stays open with the device away: `TRIAL_END` needs no fresh status, and the core refuses what it cannot send. */
private fun canEnd(enabled: Boolean, v: VregCalView) = enabled || v.phase in RUNNING

@Composable
private fun ManualSection(
    v: VregCalView,
    platform: String,
    enabled: Boolean,
    send: (suspend () -> DspiException?) -> Unit,
    cal: se.bofr.dspi.bridge.CoreVoltageCalibration
) {
    val status = v.status ?: return
    if (v.choices.isEmpty()) return
    var target by rememberSaveable { mutableStateOf(v.choices.first().code) }
    val choice: VregChoice = v.choices.firstOrNull { it.code == target } ?: v.choices.first()
    var word by rememberSaveable(target) { mutableStateOf("") }
    val typed = word.ifEmpty { null }
    SettingsSection("Manual", footer = "Save keeps the voltage across restarts. A trial lasts until the next restart.") {
        ChoiceRow("Voltage", null, v.choices.map { it.code to vregLabel(it.code, platform) }, choice.code, enabled) { target = it }
        if (choice.needsWord || choice.saveNeedsWord) WordField(v.confirmationWord, word) { word = it }
        ActionRow {
            OutlinedButton(onClick = {
                send { cal.manualTrial(choice.code, typed) }
            }, enabled = enabled && !status.trialActive) { Text("Trial") }
            Button(onClick = { send { cal.manualSet(choice.code, typed) } }, enabled = enabled) { Text("Save") }
        }
        RowDivider()
        SettingRow("Reset to the build default", vregLabel(status.defaultVreg, platform), enabled = enabled) {
            OutlinedButton(onClick = { send { cal.manualSet(0, null) } }, enabled = enabled && status.storedSel != 0) { Text("Reset") }
        }
        RowDivider()
        SettingRow("Automatic raising", "Raise the voltage by itself after repeated hangs.", enabled = enabled) {
            Switch(checked = !status.autoDisabled, onCheckedChange = { on -> send { cal.manualAuto(on) } }, enabled = enabled)
        }
        RowDivider()
        SettingRow("Forget the hang count and the last failure", null, enabled = enabled) {
            OutlinedButton(onClick = { send { cal.manualClear() } }, enabled = enabled) { Text("Clear") }
        }
    }
}

@Composable
private fun WordField(word: String, value: String, onChange: (String) -> Unit) {
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text("Type $word to go above 1.20 V") },
        singleLine = true,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp).testTag(CORE_VOLTAGE_WORD_TAG)
    )
}

@Composable
private fun ActionRow(content: @Composable () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp, androidx.compose.ui.Alignment.End)
    ) { content() }
}

/** The core's refusal as a sentence: its own text for the word and the flow's rules. */
internal fun refusalText(e: DspiException): String = when (e) {
    is DspiException.OvervoltWordRequired -> e.msg
    is DspiException.InvalidArgument -> e.msg
    else -> e.message ?: e.toString()
}

internal fun formatMinutes(seconds: Long): String {
    val m = seconds / 60
    val s = seconds % 60
    return if (s == 0L) "$m min" else String.format(Locale.ROOT, "%d:%02d", m, s)
}
