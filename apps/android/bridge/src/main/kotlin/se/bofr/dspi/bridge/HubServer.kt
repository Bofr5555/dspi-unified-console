// WP P2AN: the hub's own families for the Android screens (PLAN-P2: shared
// snapshots, the hub's history, alarm notices, Identify). Every rule is the
// core's (hub_server.rs, P2CC1/P2CC2): which families the hub has
// (`hub_caps`), the merge of both sides' snapshots with the hub's winning,
// the alarm's notice row, Identify through the hub or the device's own
// EXT_IDENTIFY. This carries the calls onto the bridge's FFI thread, keeps
// the caps and the alarms observable and hands the hub's events
// (`event.added`, `snapshot.changed`, `alarm.changed`) to the main thread.
// A hub without a family shows nothing of it: every call answers
// `unsupported` there and sends nothing.
package se.bofr.dspi.bridge

import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.EventEntry
import se.bofr.dspi.core.HubAlarm
import se.bofr.dspi.core.HubCaps
import se.bofr.dspi.core.HubEventPage
import se.bofr.dspi.core.HubEventRecord
import se.bofr.dspi.core.HubServerEvent
import se.bofr.dspi.core.HubServerListener
import se.bofr.dspi.core.HubSnapshotPut
import se.bofr.dspi.core.IdentifyInfo
import se.bofr.dspi.core.Notice
import se.bofr.dspi.core.SnapshotInfo
import se.bofr.dspi.core.SnapshotMergedList

/** A failed call to the hub, in words for the user (the core's notice where it has one). */
class HubServerException(message: String, cause: Throwable? = null) : Exception(message, cause)

/** [DspiBridge.hubServer]. Suspending members run on the bridge's FFI thread; call them from the main thread. */
interface HubServer {
    /** The hub's families (`hub_caps`, no I/O), read again on every device change; [NO_CAPS] off a hub. */
    val caps: StateFlow<HubCaps>

    /**
     * The hub's alarms that are on (raised or acknowledged) and concern the
     * open device or no one device (the audio link, the hub), newest first.
     * Empty without the `alarms` cap. Kept by `alarm.list` and `alarm.changed`.
     */
    val alarms: StateFlow<List<HubAlarm>>

    /** Each alarm the first time it is seen raised (not acknowledged yet), the last few replayed: the app's notice. On main. */
    val raised: SharedFlow<HubAlarm>

    /** A record added to the open device's history on the hub while [watchEvents] is on. On main. */
    val eventAdded: SharedFlow<HubEventRecord>

    /** A snapshot on the hub changed (`snapshot.changed`: its id). On main. */
    val snapshotsChanged: SharedFlow<String>

    /** Read the alarms again (`alarm.list`); nothing without the cap. */
    suspend fun refreshAlarms()

    /** What to tell the user about [alarm] (`hub_alarm_notice`, the core's row); null without a core. */
    suspend fun alarmNotice(alarm: HubAlarm): Notice?

    /** Acknowledge [id] (`alarm.ack`; Control or Admin). */
    suspend fun acknowledge(id: String): Result<HubAlarm>

    /**
     * Identify the open device (`hub_identify`): through a hub with the
     * `identify` cap the hub sends it, connected directly it is the device's
     * own EXT_IDENTIFY. [seconds] 0 stops the blink; [toneOutput] only when the
     * user asked for sound.
     */
    suspend fun identify(seconds: Int, toneOutput: Int? = null): Result<IdentifyInfo?>

    /** Identify can be offered: a device is open and, through a hub, the hub has the cap. */
    val identifyOffered: Boolean
        get() = caps.value.let { !it.onHub || it.identify }

    /** This console's own event log of the open device, newest first, at most [limit] (`event_log_read`). Never fails. */
    suspend fun localEvents(limit: Long = EVENTS_PAGE): List<EventEntry>

    /** The open device's history on the hub (`event.list`): with [afterId] 0 the newest page. */
    suspend fun hubEvents(afterId: Long = 0, limit: Long = EVENTS_PAGE): Result<HubEventPage>

    /** Have the hub send the open device's new records ([eventAdded]) while [on]. Best effort. */
    suspend fun watchEvents(on: Boolean)

    /** Both sides' snapshots of the open device (`snapshot_list_merged`). Never fails. */
    suspend fun mergedSnapshots(): SnapshotMergedList

    /** "Save to the hub": file this console's snapshot [id] on the hub. */
    suspend fun saveToHub(id: String): Result<HubSnapshotPut>

    /** "Copy here": file the hub's snapshot [id] in this console's own store. */
    suspend fun copyHere(id: String): Result<SnapshotInfo>

    suspend fun renameOnHub(id: String, name: String): Result<SnapshotInfo>

    suspend fun noteOnHub(id: String, note: String): Result<SnapshotInfo>

    /** Remove [id] from the hub (Admin); this console's own copy stays. */
    suspend fun deleteFromHub(id: String): Result<Unit>

    /** Notice row [key] as one line ("Title: reason"), null when the core has none. */
    suspend fun noticeLine(key: String): String?

    companion object {
        /** One page of history. */
        const val EVENTS_PAGE = 200L

        /** `MergedSnapshot.location` (the core's `WHERE_*`). */
        const val WHERE_LOCAL = "local"
        const val WHERE_HUB = "hub"
        const val WHERE_BOTH = "both"

        /** `HubAlarm.state`. */
        const val ALARM_RAISED = "raised"
        const val ALARM_CLEARED = "cleared"

        /** `HubServerEvent.kind`. */
        const val EVENT_ADDED = "event.added"
        const val SNAPSHOT_CHANGED = "snapshot.changed"
        const val ALARM_CHANGED = "alarm.changed"

        val NO_CAPS = HubCaps(
            onHub = false, inventory = false, identify = false, health = false, events = false, hubSnapshots = false,
            paramApi = false, hubStats = false, alarms = false, outbound = false, schedule = false, mqtt = false,
            apiKeys = false, rest = false, caps = emptyList()
        )

        val NO_MERGE = SnapshotMergedList(emptyList(), false, "")

        /**
         * The alarms a single-device console shows: those of [serial] and those
         * about no one device; cleared ones dropped, newest first (the hub's order).
         */
        fun relevant(all: List<HubAlarm>, serial: String): List<HubAlarm> =
            all.filter { it.state != ALARM_CLEARED && (it.serial.isEmpty() || it.serial == serial) }

        /** [now] with [changed] applied (`alarm.changed`): a cleared one leaves, others are put first or replaced. */
        fun applyChange(now: List<HubAlarm>, changed: HubAlarm, serial: String): List<HubAlarm> {
            val rest = now.filter { it.id != changed.id }
            return if (relevant(listOf(changed), serial).isEmpty()) rest else listOf(changed) + rest
        }
    }

    /** For bridges without a core (test doubles): no hub, nothing to show. */
    object Unavailable : HubServer {
        private fun <T> no(): Result<T> = Result.failure(HubServerException("Not available here."))

        override val caps: StateFlow<HubCaps> = MutableStateFlow(NO_CAPS).asStateFlow()
        override val alarms: StateFlow<List<HubAlarm>> = MutableStateFlow<List<HubAlarm>>(emptyList()).asStateFlow()
        override val raised: SharedFlow<HubAlarm> = MutableSharedFlow<HubAlarm>().asSharedFlow()
        override val eventAdded: SharedFlow<HubEventRecord> = MutableSharedFlow<HubEventRecord>().asSharedFlow()
        override val snapshotsChanged: SharedFlow<String> = MutableSharedFlow<String>().asSharedFlow()
        override val identifyOffered: Boolean get() = false

        override suspend fun refreshAlarms() = Unit

        override suspend fun alarmNotice(alarm: HubAlarm): Notice? = null

        override suspend fun acknowledge(id: String): Result<HubAlarm> = no()

        override suspend fun identify(seconds: Int, toneOutput: Int?): Result<IdentifyInfo?> = no()

        override suspend fun localEvents(limit: Long): List<EventEntry> = emptyList()

        override suspend fun hubEvents(afterId: Long, limit: Long): Result<HubEventPage> = no()

        override suspend fun watchEvents(on: Boolean) = Unit

        override suspend fun mergedSnapshots(): SnapshotMergedList = NO_MERGE

        override suspend fun saveToHub(id: String): Result<HubSnapshotPut> = no()

        override suspend fun copyHere(id: String): Result<SnapshotInfo> = no()

        override suspend fun renameOnHub(id: String, name: String): Result<SnapshotInfo> = no()

        override suspend fun noteOnHub(id: String, note: String): Result<SnapshotInfo> = no()

        override suspend fun deleteFromHub(id: String): Result<Unit> = no()

        override suspend fun noticeLine(key: String): String? = null
    }
}

internal class HubServerImpl(private val bridge: DspiBridgeImpl) : HubServer {
    private val _caps = MutableStateFlow(HubServer.NO_CAPS)
    override val caps: StateFlow<HubCaps> = _caps.asStateFlow()
    private val _alarms = MutableStateFlow<List<HubAlarm>>(emptyList())
    override val alarms: StateFlow<List<HubAlarm>> = _alarms.asStateFlow()

    // A few replayed: the app's notices may start collecting after the first read (a collector skips one no longer on)
    private val _raised = MutableSharedFlow<HubAlarm>(replay = 8, extraBufferCapacity = 16)
    override val raised: SharedFlow<HubAlarm> = _raised.asSharedFlow()
    private val _eventAdded = MutableSharedFlow<HubEventRecord>(extraBufferCapacity = 64)
    override val eventAdded: SharedFlow<HubEventRecord> = _eventAdded.asSharedFlow()
    private val _snapshotsChanged = MutableSharedFlow<String>(extraBufferCapacity = 16)
    override val snapshotsChanged: SharedFlow<String> = _snapshotsChanged.asSharedFlow()

    /** Alarm ids already announced through [raised] (main thread only). */
    private val announced = HashSet<String>()

    private val core get() = bridge.core

    /** Watching the hub's history: asked again after a device change (a watch ends with the session). */
    private var watching = false

    override val identifyOffered: Boolean
        get() = bridge.connected && caps.value.let { !it.onHub || it.identify }

    init {
        // The hub's events (a core thread): handled on main
        bridge.scope.launch(bridge.main) {
            val listener = HubServerListener { e -> bridge.scope.launch(bridge.main) { onEvent(e) } }
            bridge.onFfi { runCatching { core.setHubServerListener(listener) } }
        }
        // A device opened or gone: the caps (no I/O), then the alarms and the watch (a round trip each)
        bridge.scope.launch(bridge.main) {
            bridge.deviceGeneration.collect {
                _caps.value = bridge.onFfi { runCatching { core.hubCaps() }.getOrDefault(HubServer.NO_CAPS) }
                announced.clear()
                refreshAlarms()
                if (watching) watchEvents(true)
            }
        }
    }

    /** One of the hub's events. Main thread. */
    internal fun onEvent(e: HubServerEvent) {
        val serial = bridge.selectedSerial
        when (e.kind) {
            HubServer.EVENT_ADDED -> e.record?.takeIf { it.serial.isEmpty() || serial.isEmpty() || it.serial == serial }
                ?.let { _eventAdded.tryEmit(it) }
            HubServer.SNAPSHOT_CHANGED -> if (e.serial.isEmpty() || serial.isEmpty() || e.serial == serial) {
                _snapshotsChanged.tryEmit(e.snapshot)
            }
            HubServer.ALARM_CHANGED -> e.alarm?.let { a ->
                _alarms.value = HubServer.applyChange(_alarms.value, a, serial)
                announce(listOf(a))
            }
        }
    }

    /** [raised] for each alarm on that was not announced yet. Main thread. */
    private fun announce(list: List<HubAlarm>) {
        val serial = bridge.selectedSerial
        HubServer.relevant(list, serial).filter { it.state == HubServer.ALARM_RAISED && announced.add(it.id) }
            .reversed() // oldest first
            .forEach { _raised.tryEmit(it) }
    }

    /** Runs [block] on the FFI thread; a core error becomes a failure in the user's words. */
    private suspend fun <T> call(block: () -> T): Result<T> = bridge.onFfi {
        try {
            Result.success(block())
        } catch (e: DspiException) {
            Result.failure(HubServerException(wordsOf(e), e))
        } catch (e: WritesNotAllowedException) {
            Result.failure(HubServerException(Snapshots.wordsOf(e), e))
        }
    }

    /** FFI thread: the core's notice for [e] as "Title: reason", else the plain words. */
    private fun wordsOf(e: DspiException): String = when (e) {
        is DspiException.Denied, is DspiException.Locked, is DspiException.ReadOnly, is DspiException.Link,
        is DspiException.ImageRefused, is DspiException.InstallerLock -> runCatching { core.noticeFor(e) }.getOrNull()
            ?.takeIf { it.title.isNotBlank() }
            ?.let { n ->
                listOf("${n.title}: ${n.reason}".trim().trimEnd(':'), n.detail.trim()).filter {
                    it.isNotEmpty()
                }.distinct().joinToString(" ")
            }
        else -> null
    } ?: Snapshots.wordsOf(e)

    /** A device write refused by G1 (read-only, a viewer, no device): nothing is sent. */
    private fun refused(name: String): Result<Nothing>? = if (bridge.writeRefused("hubServer.$name")) {
        Result.failure(HubServerException(Snapshots.wordsOf(WritesNotAllowedException(""))))
    } else {
        null
    }

    override suspend fun refreshAlarms() {
        if (!_caps.value.alarms) {
            _alarms.value = emptyList()
            return
        }
        val serial = bridge.selectedSerial
        val all = bridge.onFfi { runCatching { core.hubAlarms("", "active") }.getOrNull() } ?: return
        _alarms.value = HubServer.relevant(all, serial)
        announce(all)
    }

    override suspend fun alarmNotice(alarm: HubAlarm): Notice? = bridge.onFfi { runCatching { core.hubAlarmNotice(alarm, "") }.getOrNull() }

    override suspend fun acknowledge(id: String): Result<HubAlarm> {
        val r = call { core.hubAlarmAck(id) }
        r.getOrNull()?.let { _alarms.value = HubServer.applyChange(_alarms.value, it, bridge.selectedSerial) }
        return r
    }

    override suspend fun identify(seconds: Int, toneOutput: Int?): Result<IdentifyInfo?> {
        refused("identify")?.let { return it }
        return call { core.hubIdentify(seconds, toneOutput) }
    }

    override suspend fun localEvents(limit: Long): List<EventEntry> =
        bridge.onFfi { runCatching { core.eventLogRead("", 0, limit).entries }.getOrDefault(emptyList()) }.asReversed()

    override suspend fun hubEvents(afterId: Long, limit: Long): Result<HubEventPage> = call { core.hubEventsRead("", afterId, limit) }

    override suspend fun watchEvents(on: Boolean) {
        watching = on
        if (!_caps.value.events) return
        val serial = bridge.selectedSerial
        bridge.onFfi { runCatching { core.hubEventsWatch(on, serial) } }
    }

    override suspend fun mergedSnapshots(): SnapshotMergedList =
        bridge.onFfi { runCatching { core.snapshotListMerged("") }.getOrDefault(HubServer.NO_MERGE) }

    override suspend fun saveToHub(id: String) = call { core.hubSnapshotsSave(id) }

    override suspend fun copyHere(id: String) = call { core.hubSnapshotsCopyHere(id) }

    override suspend fun renameOnHub(id: String, name: String) = call { core.hubSnapshotsRename(id, name) }

    override suspend fun noteOnHub(id: String, note: String) = call { core.hubSnapshotsNote(id, note) }

    override suspend fun deleteFromHub(id: String) = call { core.hubSnapshotsDelete(id) }

    override suspend fun noticeLine(key: String): String? = bridge.onFfi {
        runCatching { core.noticeForKey(key, "") }.getOrNull()?.takeIf { it.key == key }?.let { "${it.title}: ${it.reason}".trim() }
    }
}
