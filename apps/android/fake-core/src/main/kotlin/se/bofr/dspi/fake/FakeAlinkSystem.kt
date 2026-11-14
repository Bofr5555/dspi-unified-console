// -- WP LKUI1 -- the hub's audio link installation, faked for the System screen's
// tests and previews: what dspi-core's alink_system_* give for `uconsole --demo hub
// --alink` (a sender, a fan-out, four units, four scenes), with the hub's
// rules that matter to the screen: roles, the revision guard on Save, and a sender
// that holds one scene at a time.
package se.bofr.dspi.fake

import se.bofr.dspi.core.AlinkSceneRecall
import se.bofr.dspi.core.AlinkSystemAlarm
import se.bofr.dspi.core.AlinkSystemAlign
import se.bofr.dspi.core.AlinkSystemEdits
import se.bofr.dspi.core.AlinkSystemRoom
import se.bofr.dspi.core.AlinkSystemSaved
import se.bofr.dspi.core.AlinkSystemScene
import se.bofr.dspi.core.AlinkSystemUnit
import se.bofr.dspi.core.AlinkSystemView
import se.bofr.dspi.core.AlinkSystemWrite
import se.bofr.dspi.core.DspiException

class FakeAlinkSystem {
    private companion object {
        /** The firmware's audio link delay range at the default 4 ms output latency option. */
        const val D_LOCAL_MIN = 257L
        const val D_LOCAL_MAX = 399L

        /** The system delay the demo hub takes by itself, and a unit's audio link delay before alignment. */
        const val D_AUTO = 260L
        const val UNIT_START = 300L

        /** Each unit's audio link delay against D, and its hop offset, as the demo hub's alignment gives them. */
        val PLAN = mapOf("A" to (0L to 6), "B" to (-1L to 4), "C" to (-1L to -2), "D" to (0L to 4))
    }

    /** On a hub with this installation; off, every call is [DspiException.Disconnected]. */
    @Volatile var onHub = false

    /** The session's role: `admin`, `control` or `viewer`. */
    @Volatile var role = "admin"

    /**
     * -- WP HWLK1F -- `uconsole --demo hub --alink --with-plain-unit`: one more
     * unit, P in the bar, whose firmware has no audio link, as the core shows it.
     */
    @Volatile var plainUnit = false

    /**
     * -- WP AUSB1 -- the demo hub's two USB-direct units (R "Rear L/R", H "Height"),
     * phase-locked, as the core shows them: D 192 until aligned to the system delay
     * plus the room's latency term, Height's stream 3 bus frames behind.
     */
    @Volatile var usbDirect = false

    /** The sender holds a scene for this frame (the next superframe scene is refused). */
    @Volatile var heldFrame: Long? = null

    @Volatile var revision = 1L
        private set

    private var systemDelay: Long? = null

    // -- WP FIXM1 -- the Installer's "Keep USB-direct units aligned"
    private var keepUsbAligned = false

    // -- WP AUSB2 -- the USB-direct units' modes and lock-error actions as the
    // installation asks them (unset: the units' own, phase lock; mute)
    private val usbModeWanted = HashMap<String, String?>()
    private val onLockError = HashMap<String, String>()

    /**
     * -- WP AUSB2 -- Rear L/R cannot phase-lock (its host out of range): the core
     * shows the lock error and the hub's alarm, doing what its setting says.
     */
    @Volatile var rearLockError = false
    private var aligned = false
    private var frame = 1_000_000L
    private val lipSync = HashMap<String, Long?>()
    private val latency = HashMap<String, Long?>(mapOf("C" to 120L))
    private val rooms = listOf("main" to "Main room", "terrace" to "Terrace", "bar" to "Bar")
    private val units = listOf(
        Triple("A", "Main left", "main"),
        Triple("B", "Main right", "main"),
        Triple("C", "Terrace", "terrace"),
        Triple("D", "Bar", "bar")
    )
    private val scenes = listOf(
        AlinkSystemScene("evening", "Evening", null),
        AlinkSystemScene("quiet", "Quiet inside", null),
        AlinkSystemScene("closed", "Closed", true),
        AlinkSystemScene("click", "Test click", null)
    )

    /** Another installer saved meanwhile: the revision moves. */
    @Synchronized fun changeElsewhere() {
        revision++
    }

    /** The scene the sender held has applied. */
    fun releaseHeld() {
        heldFrame = null
    }

    private fun session() {
        if (!onHub) throw DspiException.Disconnected()
    }

    private fun denied(need: String) = DspiException.Link("denied", "$need role required")

    @Synchronized fun view(): AlinkSystemView {
        session()
        // -- fixup lk1 -- the demo hub's numbers (`uconsole --demo hub --alink` over the
        // Rust fake): an audio link delay range of 257..399 at the 4 ms latency option, D 260, each
        // unit's audio link delay and hop offset as the alignment gives them, and every unit at
        // 300 frames, hop offset 0, until the audio link delays are written
        val d = systemDelay ?: D_AUTO
        val rate = 48_000L
        val ms = { f: Long -> f * 1000.0 / rate }
        val pendingUnits = if (aligned) 0L else units.size.toLong()
        val rows = mutableListOf(
            unit("tx", "Sender", "DEMO-ALINK-TX", "sender", "Sender", "", "clock", "Clock"),
            unit("fan", "Fan-out", "DEMO-ALINK-FAN", "fan_out", "Fan-out", "", "locked", "Locked").copy(hopDelayFrames = 3)
        )
        for (u in units) {
            val (id, name, room) = u
            val behindFan = id == "B" || id == "C"
            val (delta, offset) = PLAN.getValue(id)
            val planned = d + delta
            val pending = !aligned
            rows += unit(id, name, "DEMO-ALINK-$id", "unit", "Receiver", rooms.first { it.first == room }.second, "locked", "Locked").copy(
                pending = pending,
                alinkDelayFrames = planned,
                alinkDelayMs = ms(planned),
                unitAlinkDelayFrames = if (pending) UNIT_START else planned,
                alinkDelayMin = D_LOCAL_MIN,
                alinkDelayMax = D_LOCAL_MAX,
                hopOffset = offset,
                unitHopOffset = if (pending) 0 else offset,
                phaseErrorFrames = 0.04f,
                hops = if (behindFan) 1 else 0,
                measuredLatencyUs = latency[id]
            )
        }
        // -- WP AUSB1 -- the USB-direct units: their own state, D and statistics
        if (usbDirect) {
            for ((id, name, offset) in listOf(Triple("R", "Rear L/R", 144L), Triple("H", "Height", 0L))) {
                val planned = d + 6
                val pending = !aligned
                // -- WP AUSB2 --
                val action = onLockError[id] ?: "mute"
                val lockError = rearLockError && id == "R"
                val word = when {
                    !lockError -> "Phase-locked (USB)"
                    action == "play_unaligned" -> "Can't lock (USB): playing unaligned"
                    else -> "Can't lock (USB): muted"
                }
                val state = if (lockError) "rejected" else "phase_locked"
                rows += unit(id, name, "DEMO-USB-$id", "unit", "USB-direct", "Main room", state, word).copy(
                    pending = pending,
                    transport = "usb_direct",
                    usbMode = "phase_lock",
                    usbModeWanted = if (usbModeWanted.containsKey(id)) usbModeWanted[id] else "phase_lock",
                    usbLockError = lockError,
                    usbLockErrorCause = if (lockError) "the host's rate is out of range" else "",
                    usbOnLockError = action,
                    usbOnLockErrorWanted = action,
                    usbDelaySamples = planned,
                    usbDelayMs = ms(planned),
                    unitUsbDelaySamples = if (pending) 192L else planned,
                    usbDelayMin = 170L,
                    usbDelayMax = 399L,
                    hostPpm = -11.5f,
                    vcxoPpm = -11.48f,
                    phaseErrorSamples = 0.05f,
                    gaps = 0,
                    missedSamples = 0,
                    lateSamples = 0,
                    reanchors = 0,
                    timelineEnds = 0,
                    startOffsetSamples = offset,
                    usbStats = "host -11.5 ppm · VCXO -11.5 ppm · phase error +0.05 samples · gaps 0, missed 0, late 0 samples · " +
                        "re-anchors 0 · timeline ends 0" + (if (offset > 0) " · stream $offset samples early" else "") +
                        " · lock errors ${if (lockError) 1 else 0}"
                )
            }
        }
        // -- WP HWLK1F -- said as such, no audio link delay, not counted among the units that lock
        if (plainUnit) {
            rows += unit("P", "Plain unit", "DEMO-PLAIN", "unit", "No audio link", "Bar", "no_alink", "No audio link (firmware)")
        }
        val usbLocked = if (rearLockError) 1 else 2 // -- WP AUSB2 --
        val plain = (if (usbDirect) ", USB-direct $usbLocked of 2 locked" else "") +
            if (plainUnit) ", 1 without the audio link (firmware)" else ""
        val pendingAll = pendingUnits + if (usbDirect && !aligned) 2 else 0
        val admin = role == "admin"
        val control = admin || role == "control"
        return AlinkSystemView(
            revision = revision,
            installed = true,
            role = role,
            mayEdit = admin,
            mayRecall = control,
            editNote = if (admin) "" else "Changing the installation needs the hub's Admin role.",
            recallNote = if (control) "" else "Recalling a scene needs the hub's Control role.",
            systemDelayFrames = d,
            systemDelayMs = ms(d),
            systemDelayAuto = systemDelay == null,
            rateHz = rate,
            lipSyncUs = 0,
            units = rows,
            rooms = rooms.map { (id, name) -> AlinkSystemRoom(id, name, lipSync[id]) },
            scenes = scenes,
            // -- WP AUSB2 -- the hub's alarm for Rear L/R's lock error
            alarms = if (usbDirect && rearLockError) {
                val playing = onLockError["R"] == "play_unaligned"
                listOf(
                    AlinkSystemAlarm(
                        7,
                        if (playing) "warning" else "critical",
                        "Rear L/R",
                        "USB-direct unit Rear L/R can't lock — " + (if (playing) "playing unaligned" else "muted") +
                            " (the host's rate is out of range)",
                        "raised"
                    )
                )
            } else {
                emptyList()
            },
            systemMute = false,
            pendingApplyFrame = heldFrame,
            locked = 4,
            holdover = 0,
            pending = pendingAll,
            unitCount = 4,
            usbUnitCount = if (usbDirect) 2 else 0,
            usbLocked = if (usbDirect) usbLocked.toLong() else 0,
            summary = "Audio link: 4 of 4 locked" + plain + if (pendingAll > 0) ", audio link delay to write on $pendingAll" else "",
            health = when {
                usbDirect && rearLockError -> "problem" // -- WP AUSB2 --
                pendingAll > 0 || plainUnit -> "warning"
                else -> "ok"
            },
            issues = emptyList(),
            // -- WP FIXM1 --
            keepUsbAligned = keepUsbAligned,
            alignNeeded = false,
            stale = false
        )
    }

    private fun unit(id: String, name: String, serial: String, kind: String, role: String, room: String, state: String, word: String) =
        AlinkSystemUnit(
            id, name, serial, kind, role, room, true, state, word, false, null, null, null, null, null, null, null, null,
            null, null, 0, null, 0,
            // -- WP AUSB1 -- transport and the USB-direct fields
            "alink", null, null, null, null, null, null, null, null, null, null, null, null, null, null, null, "", null, "",
            // -- WP AUSB2 -- lock error, its cause, the action now and asked, firmware without it
            false, "", null, "", false
        )

    // -- WP P1DCFIX -- `system.set`'s whole answer: the hub warns about a unit without the audio link
    @Synchronized fun saveReport(revision: Long, edits: AlinkSystemEdits): AlinkSystemSaved {
        val rev = save(revision, edits)
        val warnings =
            if (plainUnit) {
                listOf("Plain unit (DEMO-PLAIN): its firmware has no audio link, so it plays unaligned.")
            } else {
                emptyList()
            }
        return AlinkSystemSaved(rev, warnings)
    }

    @Synchronized fun save(revision: Long, edits: AlinkSystemEdits): Long {
        session()
        if (role != "admin") throw denied("admin")
        if (revision != this.revision) {
            throw DspiException.Link(
                "conflict",
                "The installation changed on the hub while it was being edited (revision ${this.revision}). " +
                    "Reload it and make the changes again."
            )
        }
        if (edits.setSystemDelay) systemDelay = edits.systemDelayFrames
        // -- WP AUSB1 -- only the demo's USB-direct units take a mode
        for (m in edits.usbModes) {
            if (!usbDirect || m.unitId !in setOf("R", "H")) throw DspiException.InvalidArgument("no USB-direct unit ${m.unitId}")
            // -- WP AUSB2 -- as the core: only known words
            if (m.mode != null && m.mode !in setOf("off", "rate_lock", "phase_lock")) {
                throw DspiException.InvalidArgument("USB-direct mode ${m.mode}")
            }
            if (m.onLockError != null && m.onLockError !in setOf("mute", "play_unaligned")) {
                throw DspiException.InvalidArgument("USB-direct lock-error action ${m.onLockError}")
            }
        }
        for (m in edits.usbModes) {
            usbModeWanted[m.unitId] = m.mode
            m.onLockError?.let { onLockError[m.unitId] = it }
        }
        for (r in edits.rooms) lipSync[r.roomId] = r.lipSyncUs
        for (u in edits.units) latency[u.unitId] = u.measuredLatencyUs
        edits.keepUsbAligned?.let { keepUsbAligned = it } // -- WP FIXM1 --
        this.revision++
        return this.revision
    }

    @Synchronized fun align(): AlinkSystemAlign {
        session()
        if (role != "admin") throw denied("admin")
        aligned = true
        val written = units.map { AlinkSystemWrite(it.second, "Audio link delay", "ok", "Done", "ok") } +
            if (usbDirect) {
                listOf("Rear L/R", "Height").map { AlinkSystemWrite(it, "USB-direct mode and delay", "ok", "Done", "ok") }
            } else {
                emptyList()
            }
        if (plainUnit) {
            return AlinkSystemAlign(
                true,
                "The audio link delays were written on ${units.joinToString(", ") { it.second }} " +
                    "(each re-anchored with a short silence); skipped on Plain unit (no audio link (firmware)).",
                written + AlinkSystemWrite("Plain unit", "Audio link delay", "skipped", "Skipped", "warn")
            )
        }
        return AlinkSystemAlign(
            true,
            "The audio link delays were written. Each unit re-anchored with a short silence.",
            written
        )
    }

    @Synchronized fun recall(scene: String): AlinkSceneRecall {
        session()
        if (role == "viewer") throw denied("control")
        val s = scenes.firstOrNull { it.id == scene } ?: throw DspiException.Link("bad_request", "no scene $scene")
        if (s.id == "click" && role != "admin") {
            throw DspiException.Link("denied", "Op click on unit:A needs a role this session does not have")
        }
        frame += 10_000
        val held = heldFrame
        if (held != null) {
            return AlinkSceneRecall(
                s.id, s.name, false, frame + 4_800, 100.0, false, held,
                "${s.name} was refused: a scene is already scheduled on the sender for audio link frame $held, 90 ms after this recall. " +
                    "The sender takes one scene at a time; recall ${s.name} again after that." +
                    (if (s.systemMute != null) " System mute unchanged." else ""),
                listOf(AlinkSystemWrite("Sender", "Scene ${s.name}", "refused", "Refused", "bad"))
            )
        }
        heldFrame = frame + 4_800
        // -- WP AUSB1 -- a scene for every unit: at a USB frame on the USB-direct units
        if (usbDirect && s.id != "click") {
            val bus = (frame / 48 + 100) % 2048
            return AlinkSceneRecall(
                s.id, s.name, true, frame + 4_800, 100.0, !plainUnit, null,
                "${s.name} recalled: at USB frame $bus on Rear L/R, Height; at audio link frame ${frame + 4_800} " +
                    "through the sender on the others" + (if (plainUnit) "; applied now on Plain unit (no audio link)" else "") +
                    (if (s.systemMute == true) "; system mute on" else "") + ".",
                listOf(
                    AlinkSystemWrite("Sender", "Scene ${s.name}", "queued", "Queued", "ok"),
                    AlinkSystemWrite("Rear L/R", "Master volume", "queued", "Queued", "ok"),
                    AlinkSystemWrite("Height", "Master volume", "queued", "Queued", "ok")
                )
            )
        }
        // -- WP HWLK1F -- a scene for every unit: at once on P, never "on time" then
        if (plainUnit && s.id != "click") {
            return AlinkSceneRecall(
                s.id, s.name, true, frame + 4_800, 100.0, false, null,
                "${s.name} recalled: applied now on Plain unit (no audio link); at audio link frame ${frame + 4_800} " +
                    "through the sender on the others" + (if (s.systemMute == true) "; system mute on" else "") + ".",
                listOf(
                    AlinkSystemWrite("Sender", "Scene ${s.name}", "queued", "Queued", "ok"),
                    AlinkSystemWrite("Plain unit", "Master volume", "applied_now", "Applied now", "ok")
                )
            )
        }
        return AlinkSceneRecall(
            s.id, s.name, true, frame + 4_800, 100.0, true, null,
            "${s.name} recalled through the sender, at audio link frame ${frame + 4_800}" +
                (if (s.systemMute == true) ", system mute on" else "") + ".",
            listOf(AlinkSystemWrite("Sender", "Scene ${s.name}", "queued", "Queued", "ok"))
        )
    }
}
