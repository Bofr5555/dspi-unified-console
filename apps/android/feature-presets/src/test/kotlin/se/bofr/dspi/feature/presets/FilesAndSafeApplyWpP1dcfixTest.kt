package se.bofr.dspi.feature.presets

import java.io.File
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import se.bofr.dspi.bridge.Snapshots
import se.bofr.dspi.core.GuardPhase
import se.bofr.dspi.core.GuardedApply
import se.bofr.dspi.core.Notice
import se.bofr.dspi.core.NoticeKind
import se.bofr.dspi.fake.FakeDspiCore
import se.bofr.dspi.feature.presets.snapshots.guardBarShown
import se.bofr.dspi.feature.presets.snapshots.guardLines
import se.bofr.dspi.ui.alert.AlertHost
import se.bofr.dspi.ui.alert.AlertResponse

private fun p1dcfixGuard(phase: GuardPhase, id: Long = 1, message: String = "") =
    GuardedApply(id, phase, "Room", false, 30_000, 12_400, "", message, emptyList(), emptyList(), "")

/**
 * WP P1DCFIX: the review's Android fixups on the Presets side. F2 a lost
 * guard's bar never hides by itself; F4 "Apply with rollback" is fixed on over
 * a hub with the guard; F5 no success modals (Save as Template, a clean
 * import); F7 the countdown is the core's sentence; F8 the template files are
 * read off the caller's thread.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class FilesAndSafeApplyWpP1dcfixTest {
    /** The preset fake with a guarding hub, the template calls and the notice table's countdown row. */
    private class Fake(private val guardDefault: Boolean = false) : FakeDspiCore(presetFake().getState()) {
        var timeoutS: Long? = null
        var template = ""

        override fun applyGuardDefault() = guardDefault

        override fun configPreview(text: String, options: Long, eqChannels: Long): String =
            """{"ok":true,"count":1,"rows":[{"block":"outputs","section":"Out 1","item":"Gain","before":0,"after":-3}]}"""

        override fun configImportSelected(text: String, options: Long, eqChannels: Long): String {
            calls += "configImportSelected($options)"
            return configImport(text, options and 3L)
        }

        override fun applyGuardedImport(text: String, options: Long, eqChannels: Long, timeoutS: Long): GuardedApply {
            calls += "applyGuardedImport($options)"
            this.timeoutS = timeoutS
            return p1dcfixGuard(GuardPhase.WAITING)
        }

        override fun saveAsTemplate(name: String, appVersion: String, links: Int): String {
            calls += "saveAsTemplate($name)"
            return template
        }

        override fun templateList(userDir: String): String {
            calls += "templateList"
            return """[{"id":"builtin:k340","name":"K340","builtin":true,"platform":"RP2350","status":"Draft"}]"""
        }

        override fun templateText(id: String, userDir: String): String {
            calls += "templateText($id)"
            return template
        }

        override fun noticeForKey(key: String, detail: String): Notice =
            Notice(key, NoticeKind.INFO, "Keep these settings?", "They go back in {s} s unless you press Keep.", "Press Keep.", detail)
    }

    @Test
    fun p1dcfixALostGuardKeepsItsBarAndRevertUntilTheUsersOk() {
        val lost = p1dcfixGuard(GuardPhase.LOST, message = "The connection was lost; the device may still hold the change.")
        // Seen waiting or not (a reopened device), no timer hides it: only the user's OK does
        assertTrue(guardBarShown(lost, watched = 1, endShown = null))
        assertTrue(guardBarShown(lost, watched = -1, endShown = null))
        assertTrue("an earlier end's OK does not hide this one", guardBarShown(lost, watched = 1, endShown = 1L to GuardPhase.REVERTED))
        assertFalse(guardBarShown(lost, watched = 1, endShown = 1L to GuardPhase.LOST))
        // A kept or reverted end goes away after its time (endShown) as before
        val reverted = p1dcfixGuard(GuardPhase.REVERTED, message = "Reverted: the settings from before are back.")
        assertTrue(guardBarShown(reverted, watched = 1, endShown = null))
        assertFalse(guardBarShown(reverted, watched = 1, endShown = 1L to GuardPhase.REVERTED))
        assertFalse("not one this bar watched", guardBarShown(reverted, watched = 7, endShown = null))
        assertTrue(guardBarShown(p1dcfixGuard(GuardPhase.WAITING), watched = -1, endShown = null))
        assertFalse(guardBarShown(p1dcfixGuard(GuardPhase.IDLE), watched = 1, endShown = null))
    }

    @Test
    fun p1dcfixTheCountdownIsTheCoresSentence() = runTest {
        val b = attached(Fake())
        val words = b.snapshots.countdownWords()
        assertEquals("Keep these settings? They go back in {s} s unless you press Keep.", words)
        val (first, second) = guardLines(p1dcfixGuard(GuardPhase.WAITING), words)
        assertEquals("\"Room\"", first)
        assertEquals("Keep these settings? They go back in 13 s unless you press Keep.", second)
        assertEquals("They go back in 0 s", Snapshots.countdownLine("They go back in {s} s", -5))
        // A core without the notice table: the same English words
        assertEquals(Snapshots.COUNTDOWN_WORDS, attached(presetFake()).snapshots.countdownWords())
    }

    @Test
    fun p1dcfixRollbackIsFixedOnOverAGuardingHub() = runTest {
        val core = Fake(guardDefault = true)
        val b = attached(core)
        val host = AlertHost()
        val f = FileMenuActions(b, this, host)
        f.guardTimeoutS = 45
        f.importConfiguration(f.configurationDocument("Room.dspipreset")!!)
        runCurrent()
        assertTrue(f.preview.value!!.guarded)
        f.setGuarded(false)
        assertTrue("cannot be switched off", f.preview.value!!.guarded)
        f.applyPreview()
        runCurrent()
        assertTrue(core.calls.toString(), core.calls.any { it.startsWith("applyGuardedImport(") })
        assertFalse(core.calls.any { it.startsWith("configImportSelected(") })
        assertEquals("the app's timeout", 45L, core.timeoutS)
    }

    @Test
    fun p1dcfixOnUsbRollbackStaysTheUsersChoice() = runTest {
        val core = Fake()
        val b = attached(core)
        val host = AlertHost()
        val f = FileMenuActions(b, this, host)
        f.importConfiguration(f.configurationDocument("Room.dspipreset")!!)
        runCurrent()
        f.setGuarded(true)
        assertTrue(f.preview.value!!.guarded)
        f.setGuarded(false)
        assertFalse(f.preview.value!!.guarded)
        f.applyPreview()
        runCurrent()
        assertTrue(core.calls.any { it.startsWith("configImportSelected(") })
        // F5: a clean import is one line on the page, not a "Success" modal
        assertNull(host.alert.value)
        assertTrue(f.notice.value.isNotEmpty())
        f.clearNotice()
        assertEquals("", f.notice.value)
    }

    @Test
    fun p1dcfixSaveAsTemplateEndsInTheNoticeLine() = runTest {
        val core = Fake()
        val b = attached(core)
        val host = AlertHost()
        val f = FileMenuActions(b, this, host)
        core.template = f.configurationDocument("Room.dspipreset")!!
        val dir = File("build/p1dcfix-templates").apply { deleteRecursively() }
        f.askSaveTemplate(dir)
        host.press("Save", AlertResponse(text = "Bar"))
        // The file is written on Dispatchers.IO: wait for it (real time, at most 5 s)
        repeat(250) {
            runCurrent()
            if (f.notice.value.isNotEmpty()) return@repeat
            Thread.sleep(20)
        }
        assertNull("no success modal", host.alert.value)
        assertEquals("Template \"Bar\" saved. Open it from Open Template….", f.notice.value)
        assertTrue(File(dir, OfflineFiles.templateFileName("Bar")).isFile)
    }

    @Test
    fun p1dcfixTemplatesAreReadOffTheCallersThread() = runTest {
        val core = Fake()
        val b = attached(core)
        val f = FileMenuActions(b, this, AlertHost())
        core.template = f.configurationDocument("Room.dspipreset")!!
        f.showTemplates("")
        assertFalse("nothing read on the caller's thread", core.calls.contains("templateList"))
        runCurrent()
        assertEquals(listOf("K340"), f.templateChoice.value!!.map { it.name })
        var opened: String? = null
        f.openTemplate(f.templateChoice.value!!.single(), "") { _, name -> opened = name }
        assertFalse(core.calls.any { it.startsWith("templateText(") })
        runCurrent()
        assertTrue(core.calls.contains("templateText(builtin:k340)"))
        assertEquals(OfflineFiles.templateFileName("K340"), opened)
    }
}
