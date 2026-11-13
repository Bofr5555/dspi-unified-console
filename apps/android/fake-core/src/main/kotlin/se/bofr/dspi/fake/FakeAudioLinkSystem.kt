// -- WP LKUI1 -- the hub's audio link installation, faked for the System screen's
// tests and previews: what dspi-core's link_system_* give for `uconsole --demo hub
// --audio-link` (a sender, a fan-out, four units, four scenes), with the hub's
// rules that matter to the screen: roles, the revision guard on Save, and a sender
// that holds one scene at a time.
package se.bofr.dspi.fake

import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.LinkSceneRecall
import se.bofr.dspi.core.LinkSystemAlign
import se.bofr.dspi.core.LinkSystemEdits
import se.bofr.dspi.core.LinkSystemRoom
import se.bofr.dspi.core.LinkSystemScene
import se.bofr.dspi.core.LinkSystemUnit
import se.bofr.dspi.core.LinkSystemView
import se.bofr.dspi.core.LinkSystemWrite

class FakeAudioLinkSystem {
    private companion object {
        /** The firmware's link delay range at the default 4 ms output latency option. */
        const val D_LOCAL_MIN = 257L
        const val D_LOCAL_MAX = 399L

        /** The system delay the demo hub takes by itself, and a unit's link delay before alignment. */
        const val D_AUTO = 260L
        const val UNIT_START = 300L

        /** Each unit's link delay against D, and its hop offset, as the demo hub's alignment gives them. */
        val PLAN = mapOf("A" to (0L to 6), "B" to (-1L to 4), "C" to (-1L to -2), "D" to (0L to 4))
    }

    /** On a hub with this installation; off, every call is [DspiException.Disconnected]. */
    @Volatile var onHub = false

    /** The session's role: `admin`, `control` or `viewer`. */
    @Volatile var role = "admin"

    /**
     * -- WP HWLK1F -- `uconsole --demo hub --audio-link --with-plain-unit`: one more
     * unit, P in the bar, whose firmware has no audio link, as the core shows it.
     */
    @Volatile var plainUnit = false

    /** The sender holds a scene for this frame (the next superframe scene is refused). */
    @Volatile var heldFrame: Long? = null

    @Volatile var revision = 1L
        private set

    private var systemDelay: Long? = null
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
        LinkSystemScene("evening", "Evening", null),
        LinkSystemScene("quiet", "Quiet inside", null),
        LinkSystemScene("closed", "Closed", true),
        LinkSystemScene("click", "Test click", null)
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

    @Synchronized fun view(): LinkSystemView {
        session()
        // -- fixup lk1 -- the demo hub's numbers (`uconsole --demo hub --audio-link` over the
        // Rust fake): a link delay range of 257..399 at the 4 ms latency option, D 260, each
        // unit's link delay and hop offset as the alignment gives them, and every unit at
        // 300 frames, hop offset 0, until the link delays are written
        val d = systemDelay ?: D_AUTO
        val rate = 48_000L
        val ms = { f: Long -> f * 1000.0 / rate }
        val pendingUnits = if (aligned) 0L else units.size.toLong()
        val rows = mutableListOf(
            unit("tx", "Sender", "DEMO-LINK-TX", "sender", "Sender", "", "clock", "Clock"),
            unit("fan", "Fan-out", "DEMO-LINK-FAN", "fan_out", "Fan-out", "", "locked", "Locked").copy(hopDelayFrames = 3)
        )
        for (u in units) {
            val (id, name, room) = u
            val behindFan = id == "B" || id == "C"
            val (delta, offset) = PLAN.getValue(id)
            val planned = d + delta
            val pending = !aligned
            rows += unit(id, name, "DEMO-LINK-$id", "unit", "Receiver", rooms.first { it.first == room }.second, "locked", "Locked").copy(
                pending = pending,
                linkDelayFrames = planned,
                linkDelayMs = ms(planned),
                unitLinkDelayFrames = if (pending) UNIT_START else planned,
                linkDelayMin = D_LOCAL_MIN,
                linkDelayMax = D_LOCAL_MAX,
                hopOffset = offset,
                unitHopOffset = if (pending) 0 else offset,
                phaseErrorFrames = 0.04f,
                hops = if (behindFan) 1 else 0,
                measuredLatencyUs = latency[id]
            )
        }
        // -- WP HWLK1F -- said as such, no link delay, not counted among the units that lock
        if (plainUnit) {
            rows += unit("P", "Plain unit", "DEMO-PLAIN", "unit", "No audio link", "Bar", "no_alink", "No audio link (firmware)")
        }
        val plain = if (plainUnit) ", 1 without the audio link (firmware)" else ""
        val admin = role == "admin"
        val control = admin || role == "control"
        return LinkSystemView(
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
            rooms = rooms.map { (id, name) -> LinkSystemRoom(id, name, lipSync[id]) },
            scenes = scenes,
            alarms = emptyList(),
            systemMute = false,
            pendingApplyFrame = heldFrame,
            locked = 4,
            holdover = 0,
            pending = pendingUnits,
            unitCount = 4,
            summary = "Audio link: 4 of 4 locked" + plain + if (pendingUnits > 0) ", link delay to write on $pendingUnits" else "",
            health = if (pendingUnits > 0 || plainUnit) "warning" else "ok",
            issues = emptyList(),
            stale = false
        )
    }

    private fun unit(id: String, name: String, serial: String, kind: String, role: String, room: String, state: String, word: String) =
        LinkSystemUnit(
            id, name, serial, kind, role, room, true, state, word, false, null, null, null, null, null, null, null, null,
            null, null, 0, null, 0
        )

    @Synchronized fun save(revision: Long, edits: LinkSystemEdits): Long {
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
        for (r in edits.rooms) lipSync[r.roomId] = r.lipSyncUs
        for (u in edits.units) latency[u.unitId] = u.measuredLatencyUs
        this.revision++
        return this.revision
    }

    @Synchronized fun align(): LinkSystemAlign {
        session()
        if (role != "admin") throw denied("admin")
        aligned = true
        val written = units.map { LinkSystemWrite(it.second, "Link delay", "ok", "Done", "ok") }
        if (plainUnit) {
            return LinkSystemAlign(
                true,
                "The link delays were written on ${units.joinToString(", ") { it.second }} " +
                    "(each re-anchored with a short silence); skipped on Plain unit (no audio link (firmware)).",
                written + LinkSystemWrite("Plain unit", "Link delay", "skipped", "Skipped", "warn")
            )
        }
        return LinkSystemAlign(
            true,
            "The link delays were written. Each unit re-anchored with a short silence.",
            written
        )
    }

    @Synchronized fun recall(scene: String): LinkSceneRecall {
        session()
        if (role == "viewer") throw denied("control")
        val s = scenes.firstOrNull { it.id == scene } ?: throw DspiException.Link("bad_request", "no scene $scene")
        if (s.id == "click" && role != "admin") {
            throw DspiException.Link("denied", "Op click on unit:A needs a role this session does not have")
        }
        frame += 10_000
        val held = heldFrame
        if (held != null) {
            return LinkSceneRecall(
                s.id, s.name, false, frame + 4_800, 100.0, false, held,
                "${s.name} was refused: a scene is already scheduled on the sender for link frame $held, 90 ms after this recall. " +
                    "The sender takes one scene at a time; recall ${s.name} again after that." +
                    (if (s.systemMute != null) " System mute unchanged." else ""),
                listOf(LinkSystemWrite("Sender", "Scene ${s.name}", "refused", "Refused", "bad"))
            )
        }
        heldFrame = frame + 4_800
        // -- WP HWLK1F -- a scene for every unit: at once on P, never "on time" then
        if (plainUnit && s.id != "click") {
            return LinkSceneRecall(
                s.id, s.name, true, frame + 4_800, 100.0, false, null,
                "${s.name} recalled: applied now on Plain unit (no audio link); at link frame ${frame + 4_800} " +
                    "through the sender on the others" + (if (s.systemMute == true) "; system mute on" else "") + ".",
                listOf(
                    LinkSystemWrite("Sender", "Scene ${s.name}", "queued", "Queued", "ok"),
                    LinkSystemWrite("Plain unit", "Master volume", "applied_now", "Applied now", "ok")
                )
            )
        }
        return LinkSceneRecall(
            s.id, s.name, true, frame + 4_800, 100.0, true, null,
            "${s.name} recalled through the sender, at link frame ${frame + 4_800}" +
                (if (s.systemMute == true) ", system mute on" else "") + ".",
            listOf(LinkSystemWrite("Sender", "Scene ${s.name}", "queued", "Queued", "ok"))
        )
    }
}
