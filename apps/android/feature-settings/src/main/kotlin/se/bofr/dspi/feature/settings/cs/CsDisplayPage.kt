// The display's pages (AN8's S6, S7): a display binding opens one page with its
// wiring, the panel's live state, its settings and its dashboard pages; the
// wiring opens the shared binding editor (S3), the settings and each page their
// own editor over the core's `display` and `page` records. The panel's state is
// read once a second while this page is open, as the Mac does (WP AN11).
package se.bofr.dspi.feature.settings.cs

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.feature.settings.CsModel
import se.bofr.dspi.feature.settings.FormCaption
import se.bofr.dspi.feature.settings.Pill
import se.bofr.dspi.feature.settings.RowDivider
import se.bofr.dspi.feature.settings.SettingRow
import se.bofr.dspi.feature.settings.SettingsSaveCoordinator
import se.bofr.dspi.feature.settings.SettingsSection
import se.bofr.dspi.feature.settings.StatusLine
import se.bofr.dspi.ui.MacColors
import se.bofr.dspi.ui.writeEnabled

private val DESTRUCTIVE = Color(0xFFFF453A)

/** Where the display's stack is: its page, its wiring, its settings, or one dashboard page. */
internal sealed interface CsDisplayRoute {
    data object Hub : CsDisplayRoute

    data object Wiring : CsDisplayRoute

    data object Settings : CsDisplayRoute

    data class Page(val index: Int) : CsDisplayRoute

    companion object {
        fun save(r: CsDisplayRoute): String = when (r) {
            Hub -> "hub"
            Wiring -> "wiring"
            Settings -> "settings"
            is Page -> "page/${r.index}"
        }

        fun restore(s: String): CsDisplayRoute = when {
            s == "wiring" -> Wiring
            s == "settings" -> Settings
            s.startsWith("page/") -> s.removePrefix("page/").toIntOrNull()?.let { Page(it) } ?: Hub
            else -> Hub
        }
    }
}

/** S6 and what it opens, for the display binding in [slot]. [onClose] goes back to the list. */
@Composable
internal fun CsDisplayStack(
    bridge: DspiBridge,
    coordinator: SettingsSaveCoordinator,
    cs: CsModel,
    slot: Int,
    connected: Boolean,
    onClose: () -> Unit
) {
    var routeText by rememberSaveable(slot) { mutableStateOf(CsDisplayRoute.save(CsDisplayRoute.Hub)) }
    val route = CsDisplayRoute.restore(routeText)
    val go: (CsDisplayRoute) -> Unit = { routeText = CsDisplayRoute.save(it) }
    val json by bridge.cs.model.collectAsState()
    val display = remember(json) { CsDisplay.parse(json) }
    val drafts = remember(bridge) { CsDrafts.of(bridge) }
    val busy by coordinator.csBusy.collectAsState()
    val b = cs.bindings[slot]
    val scope = rememberCoroutineScope()
    val refreshStatus: () -> Unit = { scope.launch { bridge.cs.apply(CsDisplay.REFRESH_STATUS) } }

    when (route) {
        CsDisplayRoute.Hub -> CsDisplayHub(bridge, coordinator, cs, display, slot, connected, onClose, go)
        CsDisplayRoute.Wiring -> {
            val editor = remember(drafts, slot) { drafts.editor(CsRules.KIND_BINDING, slot, draftOf(b), cs.slotName(slot)) }
            LaunchedEffect(editor, b, cs.slotName(slot)) { editor.sync(draftOf(b), cs.slotName(slot)) }
            CsBindingEditorPage(editor, connected, busy, onClose = { go(CsDisplayRoute.Hub) }) {
                coordinator.csOp(CsModel.clearBindingOp(slot))
                drafts.drop(CsRules.KIND_BINDING, slot)
                onClose()
            }
        }
        CsDisplayRoute.Settings -> {
            val editor = remember(drafts) { drafts.editor(CsDisplay.KIND_DISPLAY, 0, display.config, "") }
            LaunchedEffect(editor, display.config) { editor.sync(display.config, "") }
            CsDisplayRecordPage(editor, "Display Settings", connected, busy, onApplied = refreshStatus, onClose = {
                go(CsDisplayRoute.Hub)
            })
        }
        is CsDisplayRoute.Page -> {
            val i = route.index
            if (!display.isActive(i)) {
                // Removed (here or elsewhere): back to the display's page
                LaunchedEffect(i) { go(CsDisplayRoute.Hub) }
                return
            }
            val page = display.pages[i]
            val editor = remember(drafts, i) { drafts.editor(CsDisplay.KIND_PAGE, i, page, "") }
            LaunchedEffect(editor, page) { editor.sync(page, "") }
            CsDisplayRecordPage(
                editor,
                "Page ${i + 1}",
                connected,
                busy,
                onApplied = refreshStatus,
                onClose = { go(CsDisplayRoute.Hub) },
                onRemove = {
                    val removing = coordinator.csOp(CsDisplay.clearPageOp(i))
                    scope.launch {
                        removing.join()
                        refreshStatus()
                    }
                    drafts.drop(CsDisplay.KIND_PAGE, i)
                    go(CsDisplayRoute.Hub)
                }
            )
        }
    }
}

/** S6: the display's wiring, live state, settings and dashboard pages. */
@Composable
private fun CsDisplayHub(
    bridge: DspiBridge,
    coordinator: SettingsSaveCoordinator,
    cs: CsModel,
    display: CsDisplayState,
    slot: Int,
    connected: Boolean,
    onClose: () -> Unit,
    go: (CsDisplayRoute) -> Unit
) {
    val busy by coordinator.csBusy.collectAsState()
    val status by coordinator.csStatus.collectAsState()
    val drafts = remember(bridge) { CsDrafts.of(bridge) }
    val b = cs.bindings[slot]
    var addError by remember { mutableStateOf<String?>(null) }
    var adding by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    BackHandler(onBack = onClose)

    // The panel's state while it is looked at; a read, so also on a read-only connection
    LaunchedEffect(connected) {
        while (connected) {
            bridge.cs.apply(CsDisplay.STATUS_READ)
            delay(CsDisplay.STATUS_POLL_MS)
        }
    }

    // The core's words for the wiring, the settings and each page
    val wiring = remember { mutableStateOf<CsDescription?>(null) }
    val settings = remember { mutableStateOf<CsDescription?>(null) }
    val pages = remember { mutableStateMapOf<Int, String>() }
    LaunchedEffect(cs, display.config, display.pages) {
        wiring.value = CsRules.parseDescription(bridge.cs.describe(CsRules.KIND_BINDING, slot, draftOf(b))).value
        settings.value = CsRules.parseDescription(bridge.cs.describe(CsDisplay.KIND_DISPLAY, 0, display.config)).value
        pages.clear()
        for (i in display.activePages) {
            CsRules.parseDescription(bridge.cs.describe(CsDisplay.KIND_PAGE, i, display.pages[i])).value?.let { pages[i] = it.summary }
        }
    }

    val st = display.status
    val free = display.firstFreePage
    LazyColumn(Modifier.fillMaxSize().testTag("cs.display")) {
        item {
            Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                Text(cs.slotName(slot).ifEmpty { CsModel.typeName(b.type) }, style = MaterialTheme.typography.titleLarge)
            }
        }
        if (!connected) item { StatusLine(CsEditor.NOT_CONNECTED, true) }
        if (status != null) item { StatusLine(status?.first, status?.second ?: false) }
        addError?.let { item { Box(Modifier.testTag("cs.error")) { StatusLine(it, true) } } }
        item {
            SettingsSection("Panel") {
                SettingRow(
                    "Wiring",
                    wiring.value?.rows?.filterIsInstance<CsRow.Pick>()?.filter { it.id != "type" }?.joinToString(" · ") { it.text },
                    Modifier.testTag("cs.display.wiring"),
                    onClick = { go(CsDisplayRoute.Wiring) }
                ) { Text("›") }
                RowDivider()
                val (text, warn) = CsDisplay.stateText(st)
                SettingRow("Panel State", CsDisplay.stateDetail(st), Modifier.testTag("cs.display.state")) {
                    Pill(
                        text,
                        if (warn) {
                            MacColors.orange
                        } else if (st.init == CsDisplay.STATE_LIVE) {
                            MacColors.green
                        } else {
                            Color.Gray
                        }
                    )
                }
            }
        }
        item {
            SettingsSection("Behavior") {
                SettingRow(
                    "Display Settings",
                    settings.value?.rows?.firstOrNull { it.id == "mode" }?.let { (it as? CsRow.Pick)?.text },
                    Modifier.testTag("cs.display.settings"),
                    onClick = { go(CsDisplayRoute.Settings) }
                ) { Text("›") }
            }
        }
        item {
            SettingsSection("Dashboard Pages") {
                if (display.activePages.isEmpty()) FormCaption("No Pages")
                display.activePages.forEachIndexed { n, i ->
                    if (n > 0) RowDivider()
                    val shown = st.page == i
                    SettingRow(
                        "${i + 1}. ${pages[i] ?: "Page ${i + 1}"}",
                        null,
                        Modifier.testTag("cs.display.page.$i"),
                        onClick = { go(CsDisplayRoute.Page(i)) }
                    ) { if (shown) Pill("On Screen", MacColors.green) }
                }
            }
        }
        item {
            Box(Modifier.testTag("cs.display.add")) {
                SettingRow(
                    "Add Page",
                    if (free == null) "All ${display.maxPages} page slots are in use." else null,
                    enabled = writeEnabled(connected && !busy && !adding && free != null)
                ) {
                    OutlinedButton(
                        onClick = {
                            val i = free ?: return@OutlinedButton
                            adding = true
                            addError = null
                            scope.launch {
                                val e = csAddDisplayPage(bridge, coordinator, drafts, i)
                                adding = false
                                if (e == null) go(CsDisplayRoute.Page(i)) else addError = e
                            }
                        },
                        enabled = writeEnabled(connected && !busy && !adding && free != null)
                    ) { Text("Add") }
                }
            }
        }
        item {
            FormCaption(
                "The panel shows the pages in order. A new page is applied at once and its editor opens; Save keeps the " +
                    "display's wiring, settings and pages across a restart."
            )
        }
    }
}

/**
 * Add: a fresh page from the core (`csNewRecord("page")`) written to page slot
 * [index] at once, as Add does for a control. Null when it took, else why not.
 */
private suspend fun csAddDisplayPage(bridge: DspiBridge, coordinator: SettingsSaveCoordinator, drafts: CsDrafts, index: Int): String? {
    coordinator.beginCsOperation()
    val error = try {
        val d = CsRules.parseDraft(bridge.cs.newRecord(CsDisplay.KIND_PAGE))
        val op = d.value?.let { CsDisplay.pageOp(index, it) }
        if (op == null) d.error ?: "The new page could not be made" else CsRules.applyError(bridge.cs.apply(op), bridge.cs::statusMessage)
    } finally {
        coordinator.endCsOperation()
    }
    if (error == null) bridge.cs.apply(CsDisplay.REFRESH_STATUS)
    if (error == null) drafts.drop(CsDisplay.KIND_PAGE, index)
    return error
}

/**
 * S7 and the settings: one `page` or `display` record. Revert and Apply as on a
 * control's page; [onRemove] (a page only) asks first.
 */
@Composable
private fun CsDisplayRecordPage(
    editor: CsEditor,
    title: String,
    connected: Boolean,
    busy: Boolean,
    onApplied: () -> Unit,
    onClose: () -> Unit,
    onRemove: (() -> Unit)? = null
) {
    val st by editor.state.collectAsState()
    var picking by rememberSaveable(editor.kind, editor.slot) { mutableStateOf<String?>(null) }
    var askDiscard by rememberSaveable { mutableStateOf(false) }
    var askRemove by rememberSaveable { mutableStateOf(false) }
    val desc = st.description
    val can = writeEnabled(connected && !busy && !st.working)

    BackHandler {
        when {
            picking != null -> picking = null
            st.dirty -> askDiscard = true
            else -> onClose()
        }
    }

    val picker = picking?.let { id -> desc?.rows?.firstOrNull { it.id == id } as? CsRow.Pick }
    if (picker != null) {
        CsPickerPage(picker) { v ->
            picking = null
            if (v != picker.value) editor.change(picker.id, v)
        }
        return
    }

    Column(Modifier.fillMaxSize()) {
        LazyColumn(Modifier.weight(1f).fillMaxWidth().testTag("cs.editor")) {
            item {
                Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                    Text(title, style = MaterialTheme.typography.titleLarge)
                    desc?.summary?.takeIf { it.isNotEmpty() }?.let {
                        Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
            if (!connected) item { StatusLine(CsEditor.NOT_CONNECTED, true) }
            st.error?.let { item { Box(Modifier.testTag("cs.error")) { StatusLine(it, true) } } }
            if (desc == null) {
                if (st.error == null) item { FormCaption("Reading…") }
            } else {
                item {
                    SettingsSection(null) {
                        desc.rows.forEachIndexed { i, row ->
                            if (i > 0) RowDivider()
                            CsRowView(row, can, onOpenPicker = { picking = it.id }) { id, v -> editor.change(id, v) }
                        }
                    }
                }
            }
            if (onRemove != null) {
                item {
                    TextButton(
                        onClick = { askRemove = true },
                        enabled = can,
                        modifier = Modifier.padding(horizontal = 16.dp).testTag("cs.remove")
                    ) { Text("Remove Page", color = if (can) DESTRUCTIVE else Color.Unspecified) }
                }
            }
        }
        HorizontalDivider()
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            if (st.working) Text("Applying…", Modifier.weight(1f).padding(top = 10.dp)) else Box(Modifier.weight(1f))
            OutlinedButton(onClick = { editor.revert() }, enabled = can && st.dirty, modifier = Modifier.testTag("cs.revert")) {
                Text("Revert")
            }
            Button(onClick = {
                editor.apply { if (it) onApplied() }
            }, enabled = can && st.dirty, modifier = Modifier.testTag("cs.apply")) { Text("Apply") }
        }
    }

    if (askDiscard) {
        AlertDialog(
            onDismissRequest = { askDiscard = false },
            title = { Text("Discard Changes?") },
            text = { Text("The changes to ${title.lowercase()} have not been applied.") },
            confirmButton = {
                TextButton(onClick = {
                    askDiscard = false
                    editor.revert()
                    onClose()
                }, modifier = Modifier.testTag("cs.discard")) { Text("Discard", color = DESTRUCTIVE) }
            },
            dismissButton = { TextButton(onClick = { askDiscard = false }) { Text("Keep Editing") } }
        )
    }
    if (askRemove && onRemove != null) {
        AlertDialog(
            onDismissRequest = { askRemove = false },
            title = { Text("Remove $title?") },
            text = { Text("The page leaves the panel at once. Save keeps the change across a restart.") },
            confirmButton = {
                TextButton(onClick = {
                    askRemove = false
                    onRemove()
                }, modifier = Modifier.testTag("cs.remove.confirm")) { Text("Remove", color = DESTRUCTIVE) }
            },
            dismissButton = { TextButton(onClick = { askRemove = false }) { Text("Cancel") } }
        )
    }
}
