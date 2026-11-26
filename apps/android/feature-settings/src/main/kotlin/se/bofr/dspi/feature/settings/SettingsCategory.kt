// Port of DSPi-Console-b5:DSPi Console/DSPi_ConsoleApp.swift `SettingsCategory`
// (private enum beside SettingsView). Page split on a phone follows
// DSPi-Console-Linux qt/qml/settings/pages/*Page.qml (one page per category).
package se.bofr.dspi.feature.settings

/** One selectable Settings page: title, badge tint, group. */
enum class SettingsCategory(val title: String, val tint: Long, val id: String) {
    GENERAL("About", 0xFF646971, "general"),
    ADVANCED("Advanced", 0xFF727880, "advanced"),
    NETWORKING("Networking", 0xFF2F7FBF, "networking"),
    GRAPHING("Graphing", 0xFF587A89, "graphing"),
    SPECTRUM("Spectrum Analyser", 0xFF4C808A, "spectrum"),
    OVERVIEW("Overview", 0xFF627080, "overview"),
    SPDIF_INPUT("Inputs", 0xFF04856F, "spdifInput"),
    OUTPUT_ASSIGNMENT("Outputs", 0xFF0278C7, "outputAssignment"),

    // -- WP UAN -- (UXP B8) which output drives what
    SPEAKERS("Speakers", 0xFF3C7A5A, "speakers"),
    I2S_CONFIG("I2S Configuration", 0xFFBA3822, "i2sConfig"),
    GLOBAL_PARAMS("Global Parameters", 0xFF807701, "globalParams"),
    CONTROL_SURFACES("Control Surfaces", 0xFF8354A0, "controlSurfaces"),
    CONTROL_INTERFACES("Control Interfaces", 0xFF8F60AD, "controlInterfaces"),
    CHANNEL_GROUPS("Channel Groups", 0xFF6C64B7, "channelGroups"),
    MACROS("Macros", 0xFF9E528D, "macros"),
    AUX_OUTPUTS("Auxiliary Outputs", 0xFF7A5AAC, "auxOutputs"),

    // -- WP P1AN2 -- the Installer mode's pages (UI-MODES section 2: limits and power-on)
    INSTALLER("Installer", 0xFFB0602A, "installer"),
    POWER_ON("At Power-On", 0xFF9A6A1E, "powerOn"),

    // -- WP P2AN -- the device's events: this console's log and the hub's history
    EVENTS("History", 0xFF5C6B78, "events");

    /** Hidden when the connected hardware/firmware has no such page (macOS `isAvailable`). */
    fun isAvailable(f: SettingsAvailability): Boolean = when {
        // -- WP P1AN2 -- the locked-user state: About and the installer's note only
        f.lockedUser -> this == GENERAL || this == INSTALLER
        // -- WP PGAND --
        f.alinkSender -> this == GENERAL || (this == NETWORKING && f.networkingAvailable)
        this == INSTALLER || this == POWER_ON -> f.installerMode
        else -> availableOnDevice(f)
    }

    private fun availableOnDevice(f: SettingsAvailability): Boolean = when (this) {
        I2S_CONFIG -> f.platformName != "STM32H723"
        SPDIF_INPUT -> f.inputSourceSupported
        CONTROL_INTERFACES -> f.controlInterfacesSupported
        CONTROL_SURFACES -> f.controlSurfacesSupported || !f.connected
        CHANNEL_GROUPS -> f.csGroupsSupported
        MACROS -> f.csMacrosSupported
        AUX_OUTPUTS -> f.csAuxSupported
        NETWORKING -> f.networkingAvailable
        EVENTS -> f.connected // -- WP P2AN --
        else -> true
    }

    companion object {
        /**
         * Sidebar groups in display order. "Software Update" is not ported
         * (PORT-MAP: Play/F-Droid updates); "Networking" is the DSPi Link
         * client half only (no hub on a phone).
         */
        val groups: List<Pair<String, List<SettingsCategory>>> = listOf(
            "Application" to listOf(GENERAL, ADVANCED, NETWORKING),
            "Display" to listOf(GRAPHING, SPECTRUM),
            // -- WP P2AN -- History; -- WP UAN -- Speakers
            "System" to listOf(OVERVIEW, SPDIF_INPUT, OUTPUT_ASSIGNMENT, SPEAKERS, I2S_CONFIG, GLOBAL_PARAMS, EVENTS),
            "Control" to listOf(CONTROL_SURFACES, CONTROL_INTERFACES, CHANNEL_GROUPS, MACROS, AUX_OUTPUTS),
            // -- WP P1AN2 --
            "Installer" to listOf(INSTALLER, POWER_ON)
        )

        fun ofId(id: String): SettingsCategory? = entries.firstOrNull { it.id == id }
    }
}

/** The device facts [SettingsCategory.isAvailable] reads. */
data class SettingsAvailability(
    val connected: Boolean = false,
    val platformName: String = "",
    val inputSourceSupported: Boolean = true,
    val controlInterfacesSupported: Boolean = false,
    val controlSurfacesSupported: Boolean = false,
    val csGroupsSupported: Boolean = false,
    val csMacrosSupported: Boolean = false,
    val csAuxSupported: Boolean = false,
    /** The app provides a DSPi Link client (`LocalNetworking`). */
    val networkingAvailable: Boolean = false,
    // -- WP P1AN2 -- the UI mode: Installer shows its pages; the locked-user state shows About and the note
    val installerMode: Boolean = false,
    val lockedUser: Boolean = false,
    // -- WP PGAND -- the alink sender: this app's own pages only, no DSP page
    val alinkSender: Boolean = false
)
