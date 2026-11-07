// Port of DSPi Console/Onboarding/GettingStartedView.swift (ext-channel-116) and
// DSPi-Console Qt qml/GettingStartedView.qml on the core's words (CG10).
// Android deviation: the board stage does not run its own installer. It says
// where the connected board stands and its button opens Firmware Update, which
// already finds a board in BOOTSEL and writes it; back from there returns here.
package se.bofr.dspi.feature.onboarding

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import se.bofr.dspi.ui.DspiIcons
import se.bofr.dspi.ui.DspiRoutes
import se.bofr.dspi.ui.MacColors

/** Test tag of the wizard's root. */
const val GETTING_STARTED_TAG = "getting-started"

/** Where the connected board's firmware stands against the one this app ships. */
enum class BoardMatch { NONE, MATCH, DEVICE_OLDER, DEVICE_NEWER }

/** What the board stage shows: the [match], and the two versions its text names. */
data class BoardStatus(val match: BoardMatch = BoardMatch.NONE, val deviceVersion: String = "", val bundledVersion: String = "")

/** The board stage's heading and card for [match] (Qt `headerFor`/`cardFor`, before an install starts). */
internal object BoardWords {
    fun headerId(match: BoardMatch): String = when (match) {
        BoardMatch.NONE -> "board.prepare"
        BoardMatch.MATCH -> "board.current"
        BoardMatch.DEVICE_OLDER -> "board.older"
        BoardMatch.DEVICE_NEWER -> "board.newer"
    }

    fun cardId(match: BoardMatch): String = when (match) {
        BoardMatch.NONE -> "card.waiting"
        BoardMatch.MATCH -> "card.already_installed"
        BoardMatch.DEVICE_OLDER -> "card.update_available"
        BoardMatch.DEVICE_NEWER -> "card.downgrade"
    }

    /** The card's button; with no board, the one the ready card offers. None once the firmware matches. */
    fun action(stage: OnboardingStage, match: BoardMatch): String? = when (match) {
        BoardMatch.MATCH -> null
        BoardMatch.NONE -> stage.card("card.ready")?.action
        else -> stage.card(cardId(match))?.action
    }
}

/**
 * Entry point for [DspiRoutes.GETTING_STARTED]. [onInstall] opens Firmware Update; [onDone]
 * is Skip or Finish, after which the caller records setup as done and leaves. System back
 * steps back through the stages, and on the first one leaves as Skip does.
 */
@Composable
fun GettingStartedScreen(
    words: GettingStartedWords,
    board: BoardStatus,
    onInstall: () -> Unit,
    onDone: () -> Unit,
    modifier: Modifier = Modifier
) {
    val stages = words.stages
    var index by rememberSaveable { mutableIntStateOf(0) }
    val stageIndex = index.coerceIn(0, (stages.size - 1).coerceAtLeast(0))
    val stage = stages.getOrNull(stageIndex)
    val last = stageIndex >= stages.size - 1
    BackHandler { if (stageIndex > 0) index = stageIndex - 1 else onDone() }

    Surface(modifier.fillMaxSize().testTag(GETTING_STARTED_TAG), color = MaterialTheme.colorScheme.background) {
        Column(Modifier.fillMaxSize()) {
            Column(
                Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainer).padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp)
            ) {
                Text(words.title, style = MaterialTheme.typography.titleLarge)
                if (stages.isNotEmpty()) {
                    Text(
                        OnboardingText.fill(words.stepCounter, mapOf("current" to "${stageIndex + 1}", "total" to "${stages.size}")),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    StageStrip(stages.map { it.label }, stageIndex)
                }
            }
            Column(
                Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()).padding(20.dp),
                verticalArrangement = Arrangement.spacedBy(14.dp)
            ) {
                when {
                    stage == null -> Unit
                    stage.id == "board" -> BoardStage(stage, board, onInstall)
                    else -> TextStage(stage)
                }
            }
            HorizontalDivider()
            Row(Modifier.fillMaxWidth().padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                if (!last) TextButton(onClick = onDone) { Text(words.button("skip")) }
                Spacer(Modifier.weight(1f))
                if (stageIndex > 0) {
                    OutlinedButton(onClick = { index = stageIndex - 1 }) { Text(words.button("back")) }
                    Spacer(Modifier.width(8.dp))
                }
                // Qt/macOS: the board stage moves on once the firmware is there; Skip is always offered
                val canContinue = stage?.id != "board" || board.match == BoardMatch.MATCH
                if (last) {
                    Button(onClick = onDone) { Text(words.button("finish")) }
                } else {
                    Button(onClick = { index = stageIndex + 1 }, enabled = canContinue) { Text(words.button("continue")) }
                }
            }
        }
    }
}

@Composable
private fun TextStage(stage: OnboardingStage) {
    Text(stage.title, style = MaterialTheme.typography.headlineSmall)
    stage.paragraphs.forEach { Text(it.text, style = MaterialTheme.typography.bodyLarge) }
    stage.rows.forEach { row ->
        Row(verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            val icon = iconFor(row.icon)
            if (icon != null) {
                Icon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(22.dp))
            } else {
                Text("•", color = MaterialTheme.colorScheme.primary)
            }
            Text(row.text, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
        }
    }
}

@Composable
private fun BoardStage(stage: OnboardingStage, board: BoardStatus, onInstall: () -> Unit) {
    val values = mapOf("device_version" to board.deviceVersion, "bundled_version" to board.bundledVersion)
    stage.header(BoardWords.headerId(board.match))?.let { h ->
        Text(h.title, style = MaterialTheme.typography.headlineSmall)
        Text(h.blurb, style = MaterialTheme.typography.bodyLarge)
    }
    val card = stage.card(BoardWords.cardId(board.match))
    val action = BoardWords.action(stage, board.match)
    Column(
        Modifier.fillMaxWidth()
            .background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(8.dp))
            .padding(14.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp)
    ) {
        val (glyph, tint) = when (board.match) {
            BoardMatch.NONE -> "⏚" to MaterialTheme.colorScheme.onSurfaceVariant
            BoardMatch.MATCH -> "✓" to MacColors.green
            BoardMatch.DEVICE_OLDER -> "↑" to MaterialTheme.colorScheme.primary
            BoardMatch.DEVICE_NEWER -> "↓" to MacColors.orange
        }
        Text(glyph, color = tint, style = MaterialTheme.typography.headlineMedium)
        if (card != null) {
            Text(OnboardingText.fill(card.title, values), fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center)
            Text(
                OnboardingText.fill(card.text, values),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.widthIn(max = 420.dp)
            )
        }
        if (action != null) {
            if (board.match == BoardMatch.DEVICE_NEWER) {
                OutlinedButton(onClick = onInstall) { Text(action) }
            } else {
                Button(onClick = onInstall) { Text(action) }
            }
        }
    }
}

/** A labelled dot per stage, the current one bold (macOS step strip). */
@Composable
private fun StageStrip(labels: List<String>, current: Int) {
    val accent = MaterialTheme.colorScheme.primary
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        labels.forEachIndexed { i, label ->
            if (i != 0) {
                Box(
                    Modifier.weight(1f).padding(horizontal = 6.dp).height(2.dp)
                        .background(if (i <= current) accent.copy(alpha = 0.6f) else muted.copy(alpha = 0.2f))
                )
            }
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                Box(Modifier.size(12.dp).background(if (i <= current) accent else muted.copy(alpha = 0.25f), CircleShape))
                Text(
                    label,
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight = if (i == current) FontWeight.Bold else FontWeight.Normal,
                    color = if (i == current) Color.Unspecified else muted
                )
            }
        }
    }
}

/** The core's icon names, on the app's stroke icons; null for one this app has no icon for. */
internal fun iconFor(name: String?): ImageVector? = when (name) {
    "connect" -> DspiIcons.link
    "verified" -> DspiIcons.check
    "sliders" -> DspiIcons.sliders
    "outputs" -> DspiIcons.chip
    "speaker" -> DspiIcons.speaker
    "help" -> DspiIcons.info
    else -> null
}
