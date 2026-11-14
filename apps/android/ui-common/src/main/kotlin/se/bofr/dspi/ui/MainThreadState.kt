package se.bofr.dspi.ui

import android.os.Looper
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

// -- WP FLAKE2 --
/**
 * Run [block], a write of Compose state, on the main thread. An effect (LaunchedEffect,
 * produceState) resumes after a bridge call wherever its dispatcher leaves it: on the phone
 * that is the main thread, where this runs [block] at once; under Compose's test dispatcher
 * it is the bridge's FFI thread, and a snapshot write there, racing a composition on the
 * main thread, was not always seen (AN9's summaries, review FLAKE1). Use it for every state
 * write that follows a suspending bridge call in an effect.
 */
suspend fun <T> onMainThread(block: () -> T): T =
    if (Looper.myLooper() == Looper.getMainLooper()) block() else withContext(Dispatchers.Main.immediate) { block() }
