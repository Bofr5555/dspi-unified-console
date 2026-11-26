package se.bofr.dspi.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

// -- WP INTE -- REVE C1: a number field (ValueField, the compact fields' editor) writes only text the user typed
class ValueFieldTypedTextTest {
    @Test
    fun tapThenLeaveWritesNothing() {
        // The editor opens pre-filled with the formatted value; Done or focus loss with it untouched
        assertNull(editedText("-3.26", "-3.26"))
        assertNull(editedText(compactGainEditText(-3.255f), compactGainEditText(-3.255f)))
        assertNull(editedText("", ""))
    }

    @Test
    fun typedTextIsWritten() {
        assertEquals("-3", editedText("-3.26", "-3"))
        assertEquals("", editedText("1.0", "")) // cleared: the caller's parse rejects it and the field reverts
        assertEquals("1.00", editedText("1.0", "1.00"))
    }
}
