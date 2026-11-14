package se.bofr.dspi.ui.alert

import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** WP P1AN2 (U5): a success goes to the notice line, never a modal; several lines become one. */
class NoticeLineWpP1an2Test {
    @Test
    fun wpP1an2ASuccessIsANoticeNotAModal() = runTest {
        val host = AlertHost()
        var posted = ""
        backgroundScope.launch { posted = host.notices.first() }
        runCurrent()
        host.showSuccess("Database updated successfully.\nEntries: 3")
        runCurrent()
        assertNull("no modal", host.alert.value)
        assertEquals("Database updated successfully. Entries: 3", host.notice.value)
        assertEquals(host.notice.value, posted)
        // Errors and questions stay modal
        host.showError("No device connected.")
        assertEquals("Error", host.alert.value?.messageText)
    }
}
