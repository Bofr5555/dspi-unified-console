// Port of DSPi-Console-b5:DSPi Console/DSPi_ConsoleApp.swift `GeneralSettingsTab` ("About").
// Android addition: the Licence section (LicensesView, NOTICE.md).
// Who made what, the links and the credits are the core's About table (AboutInfo.kt, WP AN26).
package se.bofr.dspi.feature.settings

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** Brand colours of the links in the About table, by stable id, as on macOS. Words and URLs are the table's. */
private val LINK_COLORS = mapOf(
    "youtube" to Color.Red,
    "github" to Color(0xFFB8B8B8),
    "discord" to Color(0xFF5865F2),
    "patreon" to Color(0xFFFF9500),
    "ko_fi" to Color(0xFF719EBC)
)

/** A row's click for [url]; null, so the row isn't clickable, when the table gives none. */
private fun UriHandler.opener(url: String?): (() -> Unit)? {
    if (url == null) return null
    return { runCatching { openUri(url) } }
}

/** The About page's own sub-pages: null = About, [LICENSES] = overview, else a bundled text's asset name. */
private const val LICENSES = "licenses"

@Composable
fun GeneralSettingsTab() {
    var page by rememberSaveable { mutableStateOf<String?>(null) }
    BackHandler(enabled = page != null) { page = if (page == LICENSES) null else LICENSES }
    when (val p = page) {
        null -> AboutPage(onOpenLicenses = { page = LICENSES })
        LICENSES -> LicensesView(onOpenText = { page = it.asset })
        else -> {
            val text = (THIRD_PARTY.mapNotNull { it.text } + GPL3).firstOrNull { it.asset == p }
            if (text == null) page = LICENSES else LicenseTextView(text, onBack = { page = LICENSES })
        }
    }
}

@Composable
private fun AboutPage(onOpenLicenses: () -> Unit) {
    val context = LocalContext.current
    val uri = LocalUriHandler.current
    val version = remember {
        runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull() ?: "—"
    }
    val readAbout = LocalAboutJson.current
    val about = remember(readAbout) { runCatching { parseAbout(readAbout()) }.getOrDefault(AboutInfo()) }
    SettingsForm {
        item {
            SettingsSection(null) {
                Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text("🔊", fontSize = 32.sp)
                    Spacer(Modifier.width(14.dp))
                    Column {
                        Text("DSPi Console", style = MaterialTheme.typography.titleMedium)
                        Text(
                            "USB Audio DSP Controller",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Text(
                            "Version $version",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        about.creditLines.forEach { line ->
                            Text(
                                line,
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                    }
                }
            }
        }
        if (about.makers.isNotEmpty()) {
            item {
                SettingsSection("Made by") {
                    about.makers.forEach { maker ->
                        SettingRow(maker.name, maker.made.joinToString(", "), onClick = uri.opener(maker.url))
                    }
                }
            }
        }
        if (about.links.isNotEmpty()) {
            item {
                // WP CG29: the heading and note are the table's; they say whose links these are.
                SettingsSection(about.linksTitle) {
                    about.linksNote?.let { FormCaption(it) }
                    FormCaption(
                        "DSPi Firmware and Console are free, open-source software that I develop in my spare time. " +
                            "Contributions of any kind - code, feedback, funding, or otherwise - are always immensely appreciated."
                    )
                    about.links.forEach { link ->
                        SettingRow(link.title, link.detail, onClick = uri.opener(link.url)) {
                            Text("●", color = LINK_COLORS[link.id] ?: MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    }
                }
            }
        }
        if (about.credits.isNotEmpty()) {
            item {
                SettingsSection("Credits") {
                    about.credits.forEach { credit ->
                        SettingRow(credit.name, credit.what, onClick = uri.opener(credit.url))
                    }
                }
            }
        }
        item {
            SettingsSection("Licence") {
                SettingRow("Licences", "GPL-3.0 with additional terms · third-party notices", onClick = onOpenLicenses)
                SettingRow("Source code", APP_SOURCE_URL, onClick = { runCatching { uri.openUri(APP_SOURCE_URL) } })
                SettingRow(
                    "Privacy policy",
                    "No data collected; no network access",
                    onClick = { runCatching { uri.openUri(PRIVACY_POLICY_URL) } }
                )
            }
        }
    }
}
