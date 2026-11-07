package se.bofr.dspi.feature.presets.autoeq

import java.io.ByteArrayInputStream
import java.io.File
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL
import java.nio.file.Files
import kotlinx.coroutines.CoroutineExceptionHandler
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.DspiBridgeImpl
import se.bofr.dspi.core.AutoEqDatabaseInfo
import se.bofr.dspi.core.AutoEqFetch
import se.bofr.dspi.core.AutoEqFetchRequest
import se.bofr.dspi.core.AutoEqFetchResponse
import se.bofr.dspi.core.DspiCoreApi
import se.bofr.dspi.core.DspiException
import se.bofr.dspi.feature.presets.presetFake
import se.bofr.dspi.feature.presets.press
import se.bofr.dspi.ui.alert.AlertHost
import se.bofr.dspi.ui.alert.AlertResponse

private const val ONE = """{"version":2,"generatedAt":"2026-11-01T00:00:00Z","entryCount":1,"entries":[
{"id":"x/Y","manufacturer":"X","model":"Y","source":"oratory1990","formFactor":"in-ear","preamp":-1,"filters":[]}]}"""
private const val THREE = """{"version":1,"generatedAt":"2026-01-18T05:41:39Z","entryCount":2,"entries":[
{"id":"a/B","manufacturer":"A","model":"B","source":"crinacle","formFactor":"in-ear","preamp":0,"filters":[]},
{"id":"c/D","manufacturer":"C","model":"D","source":"crinacle","formFactor":"in-ear","preamp":0,"filters":[]}]}"""

/** WP AN18: Update Database > Download... through CG6's `autoeq_update` with a Kotlin fetch. */
@OptIn(ExperimentalCoroutinesApi::class)
class AutoEQDownloadWpAn18Test {
    /** The core's update as CG6 does it: fetch, check, write the file. Scripted here. */
    private class UpdatingCore(private val answer: (String, String, AutoEqFetch) -> AutoEqDatabaseInfo, inner: DspiCoreApi) :
        DspiCoreApi by inner {
        override fun autoeqUpdate(dbPath: String, url: String, fetch: AutoEqFetch) = answer(dbPath, url, fetch)
    }

    private fun TestScope.bridge(answer: (String, String, AutoEqFetch) -> AutoEqDatabaseInfo): DspiBridgeImpl {
        val d = StandardTestDispatcher(testScheduler)
        return DspiBridgeImpl(UpdatingCore(answer, presetFake()), backgroundScope, ffi = d, main = d, clock = { testScheduler.currentTime })
    }

    private fun userFile(): File = File(Files.createTempDirectory("an18").toFile(), "autoeq_database.json")

    private val fetch = AutoEqFetch { throw DspiException.Transfer("not used") }

    @Test fun wpAn18DownloadReplacesTheDatabaseThroughTheCore() = runTest {
        val user = userFile()
        val m = AutoEQManager({ THREE }, user, InMemoryFavoritesStore()).also { it.loadDatabase() }
        val seen = mutableListOf<Any>()
        val b = bridge { path, url, f ->
            seen += path
            seen += url
            seen += f
            File(path).also { it.parentFile?.mkdirs() }.writeText(ONE)
            AutoEqDatabaseInfo(true, null, 2, "2026-11-01T00:00:00Z", 1)
        }
        val host = AlertHost()
        val actions = AutoEQMenuActions(m, b, this, host, Dispatchers.Unconfined, fetch)
        actions.updateDatabase {}
        runCurrent()
        assertTrue("Download... is offered", host.alert.value!!.buttons.any { it.title == "Download..." })
        host.press("Download...")
        host.press("Download", AlertResponse(text = " https://example.test/autoeq_database.json "))
        runCurrent()
        assertEquals(user.path, seen[0])
        assertEquals("https://example.test/autoeq_database.json", seen[1])
        assertSame(fetch, seen[2])
        assertEquals(listOf("x/Y"), m.entries.value.map { it.id })
        assertEquals("Database updated successfully.\nEntries: 1", host.alert.value!!.informativeText)
        assertNull(host.progress.value)
    }

    @Test fun wpAn18ARefusedDownloadLeavesTheDatabase() = runTest {
        val m = AutoEQManager({ THREE }, userFile(), InMemoryFavoritesStore()).also { it.loadDatabase() }
        val b = bridge { _, _, _ -> throw DspiException.AutoEq("bad_content", "The file is not an AutoEQ database.", null) }
        val host = AlertHost()
        AutoEQMenuActions(m, b, this, host, Dispatchers.Unconfined, fetch).downloadDatabase("https://example.test/x.json")
        runCurrent()
        assertEquals("Failed to download database: The file is not an AutoEQ database.", host.alert.value!!.informativeText)
        assertEquals(2, m.entries.value.size)
        assertNull(host.progress.value)
    }

    @Test fun wpAn18AnUnexpectedFailureStillEndsTheProgress() = runTest {
        val m = AutoEQManager({ THREE }, userFile(), InMemoryFavoritesStore()).also { it.loadDatabase() }
        val b = bridge { _, _, _ -> throw IllegalStateException("not a DspiException") }
        val host = AlertHost()
        var escaped: Throwable? = null
        val scope = CoroutineScope(
            SupervisorJob() + StandardTestDispatcher(testScheduler) + CoroutineExceptionHandler { _, e -> escaped = e }
        )
        AutoEQMenuActions(m, b, scope, host, Dispatchers.Unconfined, fetch).downloadDatabase("https://example.test/x.json")
        runCurrent()
        assertTrue(escaped is IllegalStateException)
        assertNull("the overlay is gone", host.progress.value)
        assertEquals(2, m.entries.value.size)
    }

    @Test fun wpAn18OnlyHttpsIsTried() = runTest {
        var calls = 0
        val m = AutoEQManager({ THREE }, userFile(), InMemoryFavoritesStore()).also { it.loadDatabase() }
        val host = AlertHost()
        AutoEQMenuActions(
            m,
            bridge { _, _, _ ->
                calls++
                throw AssertionError()
            },
            this,
            host,
            Dispatchers.Unconfined,
            fetch
        )
            .downloadDatabase("http://example.test/x.json")
        runCurrent()
        assertEquals(0, calls)
        assertEquals("That is not an https:// address.", host.alert.value!!.informativeText)
        assertNull(HttpAutoEqFetch.parse("ftp://x/y"))
        assertNull(HttpAutoEqFetch.parse("https://"))
        assertEquals("example.test", HttpAutoEqFetch.parse(" https://example.test/a ")!!.host)
    }

    private class FakeConnection(url: URL, private val status: Int, private val body: ByteArray, private val headers: Map<String, String>) :
        HttpURLConnection(url) {
        val sent = HashMap<String, String>()

        override fun connect() = Unit

        override fun disconnect() = Unit

        override fun usingProxy() = false

        override fun getResponseCode() = status

        override fun getInputStream(): InputStream = ByteArrayInputStream(body)

        override fun getErrorStream(): InputStream = ByteArrayInputStream(body)

        override fun getHeaderField(name: String): String? = headers[name]

        override fun setRequestProperty(key: String, value: String) {
            sent[key] = value
        }
    }

    @Test fun wpAn18TheFetchPassesStatusBodyAndLimits() {
        var conn: FakeConnection? = null
        val f = HttpAutoEqFetch({ u ->
            FakeConnection(u, 403, byteArrayOf(0x7B, 0xFF.toByte()), mapOf("Retry-After" to "60", "X-RateLimit-Remaining" to "0")).also {
                conn = it
            }
        })
        val r: AutoEqFetchResponse = f.get(AutoEqFetchRequest("https://example.test/db.json", "application/json", "DSPi"))
        assertEquals(403, r.status)
        assertEquals(listOf(0x7B, 0xFF), r.body)
        assertEquals(60L, r.retryAfterS)
        assertEquals(0L, r.rateLimitRemaining)
        assertNull(r.rateLimitReset)
        assertEquals("DSPi", conn!!.sent["User-Agent"])
        assertEquals("application/json", conn!!.sent["Accept"])

        val small = HttpAutoEqFetch({ u -> FakeConnection(u, 200, ByteArray(100), emptyMap()) }, maxBytes = 10)
        assertTrue(
            runCatching {
                small.get(AutoEqFetchRequest("https://example.test/db.json", null, "DSPi"))
            }.exceptionOrNull() is DspiException.Transfer
        )
        assertTrue(
            runCatching {
                small.get(AutoEqFetchRequest("http://example.test/db.json", null, "DSPi"))
            }.exceptionOrNull() is DspiException.Transfer
        )
    }
}
