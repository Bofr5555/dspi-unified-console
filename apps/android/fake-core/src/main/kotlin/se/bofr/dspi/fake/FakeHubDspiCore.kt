// WP P2AN: the fake with a small model of a hub's own families (PLAN-P2),
// for the Android tests of shared snapshots, the hub's history, alarm
// notices and Identify: [onHub] and [hubCaps] say which families the hub
// announces, [role] is this console's role there. A family the hub lacks
// answers `unsupported` and records nothing sent. This console's own snapshot
// store is modelled too (the plain fake has none), and the merge follows the
// core's rule (snapshot.rs `snapshot_list_merged`): the hub's wins on an id
// both sides hold with different content, and the local one is listed beside
// it as a local copy. Test hooks: [hubAddEvent], [hubRaiseAlarm],
// [hubPutRemotely]; the listener gets the events as the core would.
package se.bofr.dspi.fake

import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.EventBatch
import se.bofr.dspi.core.EventEntry
import se.bofr.dspi.core.HubAlarm
import se.bofr.dspi.core.HubCaps
import se.bofr.dspi.core.HubEventPage
import se.bofr.dspi.core.HubEventRecord
import se.bofr.dspi.core.HubServerEvent
import se.bofr.dspi.core.HubServerListener
import se.bofr.dspi.core.HubSnapshotDocument
import se.bofr.dspi.core.HubSnapshotPut
import se.bofr.dspi.core.IdentifyInfo
import se.bofr.dspi.core.MergedSnapshot
import se.bofr.dspi.core.Notice
import se.bofr.dspi.core.NoticeKind
import se.bofr.dspi.core.SnapshotInfo
import se.bofr.dspi.core.SnapshotMergedList
import se.bofr.dspi.core.SnapshotRecall

open class FakeHubDspiCore(
    /** A DSPi Link session to a hub (false: connected directly). */
    @Volatile var onHub: Boolean = true,
    /** The hub's `hello.caps` of the phase-2 families. */
    @Volatile var hubCaps: Set<String> = ALL_CAPS
) : FakeDspiCore() {
    /** This console's role on the hub. */
    @Volatile var role: String = "admin"

    @Volatile var serverListener: HubServerListener? = null
        private set

    /** This console's own store and the hub's, by id. */
    val local: MutableMap<String, SnapshotInfo> = LinkedHashMap()
    val hub: MutableMap<String, SnapshotInfo> = LinkedHashMap()

    /** This console's event log and the hub's history of the device, oldest first. */
    val localLog: MutableList<EventEntry> = ArrayList()
    val hubLog: MutableList<HubEventRecord> = ArrayList()

    /** The hub's alarms, newest first. */
    val hubAlarmList: MutableList<HubAlarm> = ArrayList()

    /** The serial `hub_events_watch` was last asked for, null when not watching. */
    @Volatile var watching: String? = null

    private var nextId = 1L
    private var nextAlarm = 1

    private fun has(cap: String): Boolean = onHub && cap in hubCaps

    private fun need(cap: String, name: String, args: String = "") {
        if (!has(cap)) throw DspiException.Unsupported()
        calls += if (args.isEmpty()) name else "$name($args)"
    }

    private fun write(name: String) {
        if (role == "viewer") throw DspiException.Denied()
        calls += "$name:write"
    }

    override fun hubCaps(): HubCaps = HubCaps(
        onHub = onHub, inventory = has("inventory"), identify = has("identify"), health = has("health"), events = has("events"),
        hubSnapshots = has("hub_snapshots"), paramApi = false, hubStats = false, alarms = has("alarms"), outbound = false,
        schedule = false, mqtt = false, apiKeys = false, rest = false,
        // -- WP P3CC -- the fleet families: none on this fake
        groups = has("groups"), groupParams = has("group_params"), groupApply = has("group_apply"), rollout = has("rollout"),
        caps = if (onHub) hubCaps.sorted() else emptyList()
    )

    override fun setHubServerListener(listener: HubServerListener?) {
        serverListener = listener
    }

    override fun hubServerEvents(): List<HubServerEvent> = emptyList()

    override fun noticeForKey(key: String, detail: String): Notice = Notice(key, NoticeKind.INFO, "T:$key", "R:$key", "A:$key", detail)

    override fun noticeFor(error: DspiException): Notice = when (error) {
        is DspiException.Denied -> Notice(
            "denied",
            NoticeKind.WARNING,
            "Not allowed",
            "This console's role on the hub does not allow that.",
            "",
            ""
        )
        is DspiException.Link -> Notice("dlink.${error.code}", NoticeKind.WARNING, "Hub", error.msg, "", "")
        else -> Notice("error", NoticeKind.ERROR, "", error.message.orEmpty(), "", "")
    }

    // ── Identify ──

    /** Identify requests that reached the device or the hub: (seconds, tone). */
    val identified: MutableList<Pair<Int, Int?>> = ArrayList()

    override fun hubIdentify(seconds: Int, toneOutput: Int?): IdentifyInfo? {
        if (onHub) need("identify", "hubIdentify", "$seconds") else calls += "hubIdentify($seconds)"
        if (onHub && role == "viewer") throw DspiException.Denied()
        identified += seconds to toneOutput
        return IdentifyInfo(seconds, toneOutput, "RP2350", 2, 0x20004927L, "pico2", "")
    }

    // ── History ──

    override fun eventLogRead(serial: String, afterId: Long, limit: Long): EventBatch {
        val rows = localLog.filter { it.id > afterId }.let { if (afterId == 0L) it.takeLast(limit.toInt()) else it.take(limit.toInt()) }
        return EventBatch(rows.lastOrNull()?.id ?: afterId, 0, rows)
    }

    override fun hubEventsRead(serial: String, afterId: Long, limit: Long): HubEventPage {
        need("events", "hubEventsRead", "$afterId")
        val rows = hubLog.filter { it.id > afterId }.let { if (afterId == 0L) it.takeLast(limit.toInt()) else it.take(limit.toInt()) }
        return HubEventPage(this.serial, rows, false, hubLog.lastOrNull()?.id ?: 0)
    }

    override fun hubEventsWatch(on: Boolean, serial: String) {
        need("events", "hubEventsWatch", "$on")
        watching = if (on) serial else null
    }

    /** Test hook: this console logs [text]. */
    fun localAddEvent(text: String, kind: String = "notification") {
        val id = localLog.size + 1L
        localLog += EventEntry(id, 1_700_000_000_000 + id, kind, serial, text, "HOST", null, true, null, null, null, null, emptyList(), 1)
    }

    /** Test hook: the hub logs [text], caused by [by]; sent as `event.added` while watched. */
    fun hubAddEvent(text: String, by: String, kind: String = "notification") {
        val id = hubLog.size + 1L
        val r = HubEventRecord(id, 1_700_000_100_000 + id, kind, serial, text, "HOST", 7, by, null, null, null, null, emptyList(), 1)
        hubLog += r
        if (watching != null) serverListener?.onHubServerEvent(event("event.added", record = r))
    }

    private fun event(kind: String, record: HubEventRecord? = null, snapshot: String = "", change: String = "", alarm: HubAlarm? = null) =
        HubServerEvent(kind, alarm?.serial ?: serial, record, null, snapshot, change, alarm, null, null, null)

    // ── Alarms ──

    override fun hubAlarms(serial: String, state: String): List<HubAlarm> {
        need("alarms", "hubAlarms", state)
        return hubAlarmList.filter { (serial.isEmpty() || it.serial == serial) && (state != "active" || it.state != "cleared") }
    }

    override fun hubAlarmAck(id: String): HubAlarm {
        need("alarms", "hubAlarmAck", id)
        write("hubAlarmAck")
        val i = hubAlarmList.indexOfFirst { it.id == id && it.state != "cleared" }
        if (i < 0) throw DspiException.Link("not_found", "no such alarm on")
        val a = hubAlarmList[i]
        val now = if (a.code == "device_restarted") {
            a.copy(state = "cleared", acknowledgedBy = "Front of house", clearedAt = "2026-11-15T10:01:00Z")
        } else {
            a.copy(state = "acknowledged", acknowledgedBy = "Front of house")
        }
        hubAlarmList[i] = now
        serverListener?.onHubServerEvent(event("alarm.changed", alarm = now))
        return now
    }

    override fun hubAlarmNotice(alarm: HubAlarm, time: String): Notice = Notice(
        "alarm.${alarm.code}",
        if (alarm.severity == "critical") NoticeKind.ERROR else NoticeKind.WARNING,
        "Alarm ${alarm.code}",
        "Raised at ${time.ifEmpty { alarm.raisedAt }}",
        "Check the device.",
        alarm.message
    )

    /** Test hook: the hub raises [code] for this device ([forDevice]) or for no one device; sent as `alarm.changed`. */
    fun hubRaiseAlarm(code: String, severity: String = "warning", forDevice: Boolean = true, message: String = "it happened"): HubAlarm {
        val a = HubAlarm(
            "device:${nextAlarm++}", if (forDevice) "device" else "alink", code, if (forDevice) serial else "hub",
            if (forDevice) serial else "", severity, message, "raised", "2026-11-15T10:00:00Z", "", "", 1, "", "alarm.$code"
        )
        hubAlarmList.add(0, a)
        serverListener?.onHubServerEvent(event("alarm.changed", alarm = a))
        return a
    }

    // ── Snapshots: this console's store ──

    private fun newId(): String = (nextId++).toString().padStart(13, '0')

    private fun info(id: String, name: String, note: String = "", savedMs: Long = 1_700_000_000_000 + nextId) =
        SnapshotInfo(id, name, note, serial, savedMs, "RP2350", 8, 9, 32)

    override fun snapshotSetDir(dir: String) {
        calls += "snapshotSetDir"
    }

    override fun snapshotList(serial: String): List<SnapshotInfo> = local.values.sortedBy { it.savedMs }

    override fun snapshotCapture(name: String, note: String): SnapshotInfo = info(newId(), name, note).also {
        calls += "snapshotCapture"
        local[it.id] = it
    }

    override fun snapshotRename(id: String, name: String): SnapshotInfo =
        (local[id] ?: throw DspiException.InvalidArgument("no snapshot $id")).copy(name = name).also { local[id] = it }

    override fun snapshotSetNote(id: String, note: String): SnapshotInfo =
        (local[id] ?: throw DspiException.InvalidArgument("no snapshot $id")).copy(note = note).also { local[id] = it }

    override fun snapshotDelete(id: String) {
        local.remove(id)
    }

    override fun snapshotRecall(id: String): SnapshotRecall {
        val s = local[id] ?: throw DspiException.InvalidArgument("no snapshot $id")
        calls += "snapshotRecall($id)"
        return SnapshotRecall(s.name, true, listOf("Out 1 gain"), emptyList())
    }

    override fun applyGuardDefault(): Boolean = false

    // ── Snapshots: the hub's ──

    override fun snapshotListMerged(serial: String): SnapshotMergedList {
        val locals = snapshotList("")
        if (!has("hub_snapshots")) {
            return SnapshotMergedList(locals.map { MergedSnapshot(it, "local", false) }, false, if (onHub) "" else "")
        }
        calls += "snapshotListMerged"
        val rows = ArrayList<MergedSnapshot>()
        val ids = (hub.values + locals).sortedBy { it.savedMs }.map { it.id }.distinct()
        for (id in ids) {
            val h = hub[id]
            val l = local[id]
            when {
                h != null && l != null && h == l -> rows += MergedSnapshot(h, "both", false)
                h != null && l != null -> {
                    rows += MergedSnapshot(h, "hub", false)
                    rows += MergedSnapshot(l, "local", true)
                }
                h != null -> rows += MergedSnapshot(h, "hub", false)
                l != null -> rows += MergedSnapshot(l, "local", false)
            }
        }
        return SnapshotMergedList(rows, true, "")
    }

    override fun hubSnapshotsList(serial: String): List<SnapshotInfo> {
        need("hub_snapshots", "hubSnapshotsList")
        return hub.values.sortedBy { it.savedMs }
    }

    override fun hubSnapshotsGet(id: String): HubSnapshotDocument {
        need("hub_snapshots", "hubSnapshotsGet", id)
        val s = hub[id] ?: throw DspiException.Link("not_found", "no snapshot $id on the hub")
        return HubSnapshotDocument(s, """{"meta":{"kind":"snapshot"}}""")
    }

    private fun changed(id: String, change: String) {
        serverListener?.onHubServerEvent(event("snapshot.changed", snapshot = id, change = change))
    }

    override fun hubSnapshotsSave(id: String): HubSnapshotPut {
        need("hub_snapshots", "hubSnapshotsSave", id)
        write("hubSnapshotsSave")
        val s = local[id] ?: throw DspiException.InvalidArgument("no snapshot $id")
        val there = hub[id]
        if (there != null && there != s) throw DspiException.Link("conflict", "the hub holds a different snapshot with this id")
        hub[id] = s
        changed(id, "put")
        return HubSnapshotPut(s, there == null)
    }

    override fun hubSnapshotsCopyHere(id: String): SnapshotInfo {
        need("hub_snapshots", "hubSnapshotsCopyHere", id)
        val s = hub[id] ?: throw DspiException.Link("not_found", "no snapshot $id on the hub")
        val here = local[id]
        val copy = when {
            here == null -> s
            here == s -> return s
            else -> s.copy(id = newId())
        }
        local[copy.id] = copy
        return copy
    }

    override fun hubSnapshotsRename(id: String, name: String): SnapshotInfo {
        need("hub_snapshots", "hubSnapshotsRename", id)
        write("hubSnapshotsRename")
        val s = (hub[id] ?: throw DspiException.Link("not_found", "no snapshot $id on the hub")).copy(name = name)
        hub[id] = s
        changed(id, "rename")
        return s
    }

    override fun hubSnapshotsNote(id: String, note: String): SnapshotInfo {
        need("hub_snapshots", "hubSnapshotsNote", id)
        write("hubSnapshotsNote")
        val s = (hub[id] ?: throw DspiException.Link("not_found", "no snapshot $id on the hub")).copy(note = note)
        hub[id] = s
        changed(id, "note")
        return s
    }

    override fun hubSnapshotsDelete(id: String) {
        need("hub_snapshots", "hubSnapshotsDelete", id)
        if (role != "admin") throw DspiException.Denied()
        calls += "hubSnapshotsDelete:write"
        hub.remove(id) ?: throw DspiException.Link("not_found", "no snapshot $id on the hub")
        changed(id, "delete")
    }

    /** Test hook: another console filed [name] on the hub (with [id], or a new one); sent as `snapshot.changed`. */
    fun hubPutRemotely(name: String, id: String = newId(), note: String = ""): SnapshotInfo = info(id, name, note).also {
        hub[it.id] = it
        changed(it.id, "put")
    }

    companion object {
        val ALL_CAPS = setOf("events", "hub_snapshots", "alarms", "identify")
    }
}
