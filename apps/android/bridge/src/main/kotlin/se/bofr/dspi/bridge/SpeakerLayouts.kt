// -- WP UAN -- (UXP B8) The speaker layout (WP ROLE0): which output drives what, as the user
// confirmed it, the one safety input a phone could not give. The core keeps it per device in the
// app's files (`speakerLayoutSetDir`) and is the only judge of what a role means; this is its door.
package se.bofr.dspi.bridge

import se.bofr.dspi.core.DspiException
import se.bofr.dspi.core.SpeakerLayout

/** A speaker-layout call the core refused, in its words. */
class SpeakerLayoutException(message: String, cause: Throwable? = null) : Exception(message, cause)

/** The open device's speaker layout: the kept one, the filters' suggestion, and the user's confirmation. */
interface SpeakerLayouts {
    /** Where the layouts are kept (a folder of this app's); with none, for the core's lifetime only. */
    suspend fun setFolder(dir: String)

    /** The layout kept for the open device; null when none was set (then show [propose]). */
    suspend fun current(): Result<SpeakerLayout?>

    /** What the device's filters and routing suggest; never confirmed, never kept. */
    suspend fun propose(): Result<SpeakerLayout>

    /** Keep [layout] as the user confirmed it (its rows become the user's); returns what the core kept. */
    suspend fun confirm(layout: SpeakerLayout): Result<SpeakerLayout>

    /** Test doubles: nothing is kept. */
    object Unavailable : SpeakerLayouts {
        private fun <T> none(): Result<T> = Result.failure(SpeakerLayoutException("Not available"))

        override suspend fun setFolder(dir: String) = Unit

        override suspend fun current(): Result<SpeakerLayout?> = none()

        override suspend fun propose(): Result<SpeakerLayout> = none()

        override suspend fun confirm(layout: SpeakerLayout): Result<SpeakerLayout> = none()
    }
}

internal class SpeakerLayoutsImpl(private val bridge: DspiBridgeImpl) : SpeakerLayouts {
    private val core get() = bridge.core

    /** Runs [block] on the FFI thread; a core error becomes a failure in its words. */
    private suspend fun <T> call(block: () -> T): Result<T> = bridge.onFfi {
        try {
            Result.success(block())
        } catch (e: DspiException) {
            Result.failure(SpeakerLayoutException(Snapshots.wordsOf(e), e))
        } catch (e: NotImplementedError) {
            // A test double without layouts
            Result.failure(SpeakerLayoutException("Not available", e))
        }
    }

    override suspend fun setFolder(dir: String) {
        call { core.speakerLayoutSetDir(dir) }
    }

    override suspend fun current() = call { core.speakerLayout() }

    override suspend fun propose() = call { core.speakerLayoutPropose() }

    override suspend fun confirm(layout: SpeakerLayout) = call { core.speakerLayoutSet(layout.copy(confirmedByUser = true)) }
}
