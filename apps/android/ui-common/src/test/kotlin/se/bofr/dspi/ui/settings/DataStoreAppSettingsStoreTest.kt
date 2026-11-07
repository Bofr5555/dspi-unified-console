package se.bofr.dspi.ui.settings

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.emptyPreferences
import java.io.IOException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Review C1/C2: a broken file never crashes the app, and interim files survive a failed read. */
@OptIn(ExperimentalCoroutinesApi::class)
class DataStoreAppSettingsStoreTest {
    private class FakeStore(var failRead: Boolean = false, var failWrite: Boolean = false) : DataStore<Preferences> {
        val value = MutableStateFlow(emptyPreferences())
        override val data: Flow<Preferences> = flow {
            if (failRead) throw IOException("corrupt")
            emit(value.value)
        }

        override suspend fun updateData(transform: suspend (t: Preferences) -> Preferences): Preferences {
            if (failWrite) throw IOException("read-only")
            return transform(value.value).also { value.value = it }
        }
    }

    private class FakeLegacy(val fail: Boolean, val files: List<Map<String, Any?>> = emptyList()) : LegacyPreferenceFiles {
        var deleted = false

        override fun read(): List<Map<String, Any?>> = if (fail) throw IllegalStateException("unreadable") else files

        override fun delete() {
            deleted = true
        }
    }

    @Test
    fun unreadableAndUnwritableFileKeepsTheStoreUsable() = runTest {
        val logged = mutableListOf<String>()
        val scope = CoroutineScope(UnconfinedTestDispatcher(testScheduler))
        val st = DataStoreAppSettingsStore(FakeStore(failRead = true, failWrite = true), scope, log = { m, _ -> logged += m })
        assertEquals(AppSettings(), st.settings.value)
        st.update { it.copy(showPhase = true) }
        assertTrue("kept in memory", st.settings.value.showPhase)
        assertEquals(2, logged.size)
    }

    @Test
    fun interimFilesAreKeptWhenTheirReadFailed() = runTest {
        val scope = CoroutineScope(UnconfinedTestDispatcher(testScheduler))
        val legacy = FakeLegacy(fail = true)
        val store = FakeStore()
        DataStoreAppSettingsStore(store, scope, legacy)
        assertFalse(legacy.deleted)
        assertNull("no marker: the import is retried next start", store.value.value[AppSettingsFields.LEGACY_IMPORTED])
    }

    @Test
    fun interimFilesAreImportedThenDeleted() = runTest {
        val scope = CoroutineScope(UnconfinedTestDispatcher(testScheduler))
        val legacy = FakeLegacy(fail = false, files = listOf(mapOf("dashboardCardsPerRow" to 2)))
        val st = DataStoreAppSettingsStore(FakeStore(), scope, legacy)
        assertTrue(legacy.deleted)
        assertEquals(2, st.settings.value.dashboardCardsPerRow)
    }

    @Test
    fun interimFilesAreKeptWhenTheImportCouldNotBeWritten() = runTest {
        val scope = CoroutineScope(UnconfinedTestDispatcher(testScheduler))
        val legacy = FakeLegacy(fail = false, files = listOf(mapOf("dashboardCardsPerRow" to 2)))
        DataStoreAppSettingsStore(FakeStore(failWrite = true), scope, legacy)
        assertFalse(legacy.deleted)
    }

    // -- WP AN19 --
    @Test
    fun wpAn19_aNewInstallIsWalkedThroughSetup() = runTest {
        val scope = CoroutineScope(UnconfinedTestDispatcher(testScheduler))
        val store = FakeStore()
        val st = DataStoreAppSettingsStore(store, scope, FakeLegacy(fail = false))
        assertTrue(st.settingsLoaded.value)
        assertFalse(st.settings.value.onboardingSetupDone)
        assertEquals(
            "recorded, so the next start does not read as an existing user",
            false,
            store.value.value[AppSettingsFields.ONBOARDING_SETUP_DONE]
        )
    }

    @Test
    fun wpAn19_someoneWhoUsedTheAppBeforeSkipsSetup() = runTest {
        val scope = CoroutineScope(UnconfinedTestDispatcher(testScheduler))
        val store = FakeStore()
        store.updateData { p -> p.toMutablePreferences().also { it[AppSettingsFields.LEGACY_IMPORTED] = true } }
        val st = DataStoreAppSettingsStore(store, scope, FakeLegacy(fail = false))
        assertTrue(st.settings.value.onboardingSetupDone)
        assertEquals(true, store.value.value[AppSettingsFields.ONBOARDING_SETUP_DONE])
    }

    @Test
    fun wpAn19_aStoredAnswerIsKeptAndTheSeedSurvivesAFailedWrite() = runTest {
        val scope = CoroutineScope(UnconfinedTestDispatcher(testScheduler))
        val store = FakeStore()
        store.updateData { p ->
            p.toMutablePreferences().also {
                it[AppSettingsFields.LEGACY_IMPORTED] = true
                it[AppSettingsFields.ONBOARDING_SETUP_DONE] = false
            }
        }
        assertFalse("mid-setup stays mid-setup", DataStoreAppSettingsStore(store, scope).settings.value.onboardingSetupDone)

        val readOnly = FakeStore()
        readOnly.updateData { p -> p.toMutablePreferences().also { it[AppSettingsFields.LEGACY_IMPORTED] = true } }
        readOnly.failWrite = true
        assertTrue("in memory for this run", DataStoreAppSettingsStore(readOnly, scope).settings.value.onboardingSetupDone)
    }

    @Test
    fun wpAn19_inMemoryStoreIsLoadedAtOnce() {
        assertTrue(InMemoryAppSettingsStore().settingsLoaded.value)
    }
}
