// Pure presentation logic of the home screen, kept out of the composables so
// it is JVM-tested. Ports from DSPi-Console-b5 @ b5fdeac:
// ContentView.swift (SidebarSelection, sidebarRowHeight), MatrixMixerView.swift
// (MatrixOutput.visible), DashboardView.swift (cards, columnCount),
// Components.swift (InputLinkAlerts), FirmwareUpdateView.swift
// (FirmwareMismatchBanner text), USBDevice.swift (DSPiDevice.displayName).
// Preset labels and alerts are PRESETS' (PresetLabels.kt, PresetAlerts.kt; F1).
package se.bofr.dspi.feature.home

import se.bofr.dspi.bridge.DspiBridge
import se.bofr.dspi.core.DspState
import se.bofr.dspi.core.DspiConstants
import se.bofr.dspi.core.OutputCard

/** What the detail pane shows (macOS `SidebarSelection`); ids are APP channel ids / output indices. */
sealed interface SidebarSelection {
    data object Overview : SidebarSelection

    /** An input channel page; [ch] is its app id. */
    data class Input(val ch: Int) : SidebarSelection

    /** An output channel page; [index] is the matrix output index. */
    data class Output(val index: Int) : SidebarSelection
}

/** One output as the sidebar, matrix and dashboard name it (macOS `MatrixOutput`). */
data class MatrixOutput(val index: Int, val name: String, val descriptor: String, val isPdm: Boolean) {
    /** App channel id of this output (`eqChannel(forOutput:)`). */
    val appCh: Int get() = 2 + index

    companion object {
        /**
         * `MatrixOutput.visible(for:slotTypes:)`: RP2040 has 2 stereo slots +
         * PDM (OUT5), RP2350 4 slots + PDM (OUT9). A slot of type 1 is I2S.
         */
        fun visible(platform: String, slotTypes: List<Int>): List<MatrixOutput> {
            fun slotName(slot: Int) = if (slot < slotTypes.size && slotTypes[slot] == 1) "I2S" else "SPDIF"
            val pairs = if (platform == "RP2040") 2 else 4
            val out = mutableListOf<MatrixOutput>()
            for (p in 0 until pairs) {
                out += MatrixOutput(2 * p, "${slotName(p)} ${p + 1} L", "OUT${2 * p + 1}", false)
                out += MatrixOutput(2 * p + 1, "${slotName(p)} ${p + 1} R", "OUT${2 * p + 2}", false)
            }
            out += MatrixOutput(2 * pairs, "PDM", "OUT${2 * pairs + 1}", true)
            return out
        }
    }
}

/** The home screen's string and layout rules. */
object HomeModel {
    /** `BASE_MATRIX_INPUTS`: the routing panel shows at most the USB pair. */
    const val BASE_MATRIX_INPUTS = 2

    /** Row height (dp) tightened as inputs crowd the sidebar (`sidebarRowHeight`), scaled up for touch. */
    fun sidebarRowHeight(numInputs: Int): Int = when {
        numInputs >= 8 -> 40
        numInputs >= 6 -> 44
        else -> 48
    }

    /** `DSPiDevice.displayName`: "DSPi (last 8 of serial)". */
    fun deviceDisplayName(serial: String): String = "DSPi (${serial.takeLast(8)})"

    /** `InputLinkAlerts.showMismatchAlert` title. */
    fun linkMismatchTitle(pageInput: Int, partnerInput: Int): String = "Inputs ${pageInput + 1} and ${partnerInput + 1} don't match"

    /** `InputLinkAlerts.showMismatchAlert` informative text. */
    fun linkMismatchMessage(bandsDiffer: Boolean, preampDiffers: Boolean): String {
        val differences =
            when {
                bandsDiffer && preampDiffers -> "different filters and input trims"
                bandsDiffer -> "different filters"
                else -> "different input trims"
            }
        return "These inputs have $differences. Linking mirrors future edits across both channels, but it cannot merge settings " +
            "that already differ.\n\nChoose which channel's filters and trim to copy onto the other. This overwrites the other " +
            "channel and cannot be undone."
    }

    /** "1/2", "3/4", … for an input index (`pairLabel`). */
    fun pairLabel(input: Int): String {
        val a = minOf(input, input xor 1)
        return "${a + 1}/${a + 2}"
    }

    /**
     * FirmwareMismatchBanner text (`WIRE-FORMAT-COMPAT.md` 4.4). A wire format
     * this app cannot write ([compat]) gets the read-only wording. On this
     * app's wire format the versions are compared as the Mac does
     * (`DSPViewModel.firmwareMatch`, WP AN18): any version other than
     * [expected] gets the Mac's banner; a matching one, or one that cannot be
     * read, gets none. A build stamp after the version (`1.1.6-16-g557bce7`,
     * the K340's `bofr-main` build) is not a different version. [expected] is
     * the firmware this Console bundles; the last argument is [settingsRead]
     * of the state: false when the banner must not say that settings are shown.
     */
    fun firmwareBannerMessage(compat: Int, device: String, expected: String, settingsRead: Boolean = true): String? {
        val version = device.ifEmpty { "unknown" }
        return when (compat) {
            DspiConstants.COMPAT_FIRMWARE_TOO_NEW ->
                "This device runs firmware $version, which is newer than DSPi Console $expected. " +
                    if (settingsRead) "Some of its features may not be shown. $READ_ONLY_NOTE" else SETTINGS_UNREAD_NOTE
            DspiConstants.COMPAT_FIRMWARE_TOO_OLD ->
                "This device runs firmware $version; DSPi Console expects $expected. " +
                    if (settingsRead) "$MISSING_FEATURES_NOTE $READ_ONLY_NOTE" else SETTINGS_UNREAD_NOTE
            // -- WP AN18 --
            DspiConstants.COMPAT_OK -> when (firmwareMatch(device, expected)) {
                FirmwareOrder.NEWER ->
                    "This device runs firmware $version, which is newer than DSPi Console $expected. " +
                        "Some of its features may not be shown."
                FirmwareOrder.OLDER -> "This device runs firmware $version; DSPi Console expects $expected."
                FirmwareOrder.MATCH, null -> null
            }
            else -> null
        }
    }

    // -- WP AN18 --

    /** How a device's firmware compares with the one this Console bundles (`FirmwareMatch`). */
    enum class FirmwareOrder { MATCH, OLDER, NEWER }

    /**
     * `DSPViewModel.firmwareMatch` on the bridge's version strings ("1.1.6",
     * "1.1.6-beta5", "1.1.6 early beta"); null when either cannot be read.
     * Anything after the version and its beta (a `git describe` or other build
     * stamp) is ignored.
     */
    fun firmwareMatch(device: String, expected: String): FirmwareOrder? {
        val d = firmwareRank(device) ?: return null
        val e = firmwareRank(expected) ?: return null
        val c = compareValuesBy(d, e, { it[0] }, { it[1] }, { it[2] }, { it[3] })
        return when {
            c == 0 -> FirmwareOrder.MATCH
            c < 0 -> FirmwareOrder.OLDER
            else -> FirmwareOrder.NEWER
        }
    }

    /**
     * [major, minor, patch, betaRank]: a final release outranks its betas
     * (256), an early beta sorts below beta 1 (0), as `FirmwareVersion` does.
     */
    private fun firmwareRank(text: String): List<Int>? {
        val numeric = text.takeWhile { it.isDigit() || it == '.' }
        val fields = numeric.split('.').filter { it.isNotEmpty() }.map { it.toIntOrNull() ?: return null }
        val major = fields.firstOrNull() ?: return null
        val rest = text.drop(numeric.length).lowercase().trimStart('-', '.', ' ')
        val beta = when {
            rest.startsWith("early beta") -> 0
            rest.startsWith("beta") -> rest.drop(4).trimStart(' ').takeWhile { it.isDigit() }.toIntOrNull()?.coerceIn(1, 255) ?: 256
            else -> 256
        }
        return listOf(major, fields.getOrElse(1) { 0 }, fields.getOrElse(2) { 0 }, beta)
    }

    /**
     * The banner for the connected device, what FirmwareMismatchBanner shows;
     * null for none. The demo device is no firmware to update: only its wire
     * format can raise the banner there (WP AN18).
     */
    fun firmwareBannerMessage(bridge: DspiBridge, demo: Boolean = false): String? = firmwareBannerMessage(
        bridge.compat,
        if (demo && bridge.compat == DspiConstants.COMPAT_OK) EXPECTED_FIRMWARE else bridge.firmwareVersion,
        EXPECTED_FIRMWARE,
        settingsRead(bridge.state.value)
    )

    /**
     * Whether the mirror holds the device's settings: the core rewrites a
     * decoded image's header to this app's wire format, whatever the device
     * speaks, and leaves the zeroed default image when it could not map one.
     */
    fun settingsRead(state: DspState): Boolean = state.bulkRaw.firstOrNull()?.toInt() == DspiConstants.WIRE_FORMAT_VERSION

    /**
     * Decision G1 (owner, deliberate deviation): the sentence the banner adds
     * to the macOS wording while the bridge refuses writes.
     */
    const val READ_ONLY_NOTE = "Until the firmware matches, this app is read-only: settings are shown but cannot be changed."

    /** Older firmware that was decoded: the tools and rows it lacks follow its wire format. */
    const val MISSING_FEATURES_NOTE = "Features that firmware does not have are hidden."

    /** A wire format the core cannot map: the mirror still holds its defaults, and the screens draw them. */
    const val SETTINGS_UNREAD_NOTE =
        "This app cannot read that firmware's settings, so the values shown are defaults. Until the firmware matches, nothing can be changed."

    /** The banner button: "Details..." for newer firmware, "Update..." otherwise. */
    fun firmwareBannerButton(compat: Int, device: String = "", expected: String = EXPECTED_FIRMWARE): String =
        if (compat == DspiConstants.COMPAT_FIRMWARE_TOO_NEW ||
            (compat == DspiConstants.COMPAT_OK && firmwareMatch(device, expected) == FirmwareOrder.NEWER) // -- WP AN18 --
        ) {
            "Details..."
        } else {
            "Update..."
        }

    /**
     * The firmware this app bundles (`FirmwareVersion.CONSOLE_VERSION` of
     * :feature-firmware, and its asset names; `FirmwareBannerWpAn18Test`
     * checks they agree). WP AN18: v1.1.6, stock, per OD4.
     */
    const val EXPECTED_FIRMWARE = "1.1.6"
}

/** One dashboard card, in display order (`DashboardOverview.CardItem`). */
sealed interface DashboardCard {
    data object Input : DashboardCard

    /** -- BUG1 -- [leftName]/[rightName]: the core's column headings ("Woofers L"); null shows the outputs' labels. */
    data class OutputPair(val left: Int, val right: Int, val leftName: String? = null, val rightName: String? = null) : DashboardCard

    data class Output(val index: Int, val name: String? = null) : DashboardCard

    companion object {
        /** Narrowest a stereo pair's two lists stay readable side by side (pt -> dp). */
        const val AUTO_MIN_CARD_WIDTH = 440f
        const val SPACING = 18f

        /**
         * `DashboardOverview.cards`: the input pair, then each S/PDIF pair
         * (one side alone when the other is disabled), then PDM when enabled.
         */
        fun cards(numOutputs: Int, outputEnabled: List<Boolean>): List<DashboardCard> {
            if (numOutputs <= 0) return listOf(Input)
            fun en(i: Int) = outputEnabled.getOrElse(i) { false }
            val items = mutableListOf<DashboardCard>(Input)
            for (pair in 0 until (numOutputs - 1) / 2) {
                val l = pair * 2
                val r = l + 1
                when {
                    en(l) && en(r) -> items += OutputPair(l, r)
                    en(l) -> items += Output(l)
                    en(r) -> items += Output(r)
                }
            }
            val pdm = numOutputs - 1
            if (en(pdm)) items += Output(pdm)
            return items
        }

        /**
         * -- BUG1 -- The input pair, then the core's output cards (`DspiCore.outputCards`: paired by
         * the crossfeed pair map when the device has one, else by slot); [cards] when there are none.
         */
        fun cards(core: List<OutputCard>, numOutputs: Int, outputEnabled: List<Boolean>): List<DashboardCard> {
            if (core.isEmpty()) return cards(numOutputs, outputEnabled)
            return listOf<DashboardCard>(Input) + core.map { c ->
                val r = c.right
                // A phone's narrow columns: the short headings ("Woofers L" | "R", "R · Low")
                if (r != null) OutputPair(c.left, r, c.leftShortName, c.rightShortName) else Output(c.left, c.leftShortName)
            }
        }

        /** `columnCount(for:)`: Auto fits by width, else the user's choice; never more than [count]. */
        fun columnCount(chosen: Int, availableWidth: Float, count: Int): Int {
            val wanted = if (chosen > 0) chosen else ((availableWidth + SPACING) / (AUTO_MIN_CARD_WIDTH + SPACING)).toInt()
            return maxOf(1, minOf(wanted, count))
        }

        /** The layout panel's caption. */
        fun layoutCaption(chosen: Int): String = if (chosen ==
            0
        ) {
            "Fits as many cards per row as the window allows."
        } else {
            "Up to $chosen ${if (chosen == 1) "card" else "cards"} per row."
        }
    }
}
