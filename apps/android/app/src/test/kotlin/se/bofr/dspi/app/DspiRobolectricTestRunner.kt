package se.bofr.dspi.app

import org.junit.runners.model.FrameworkMethod
import org.robolectric.RobolectricTestRunner
import org.robolectric.internal.bytecode.InstrumentationConfiguration

/**
 * WP AN38: Robolectric with the real core's Kotlin side left outside the sandbox, for every
 * Robolectric test of this module, so the test JVM has one copy of it, as the app has on a phone.
 *
 * The Rust library keeps ONE callback vtable per listener interface for the whole process. Each copy
 * of the generated UniFFI bindings registers its own the first time it calls the library, and hands
 * Rust handles into its own handle maps. A sandbox copy beside the test JVM's own (a plain test that
 * reaches the core, such as AppNetworkingControllerTest through LinkWording) took the vtables from
 * whichever copy registered first: every later listener of the other copy threw "Invalid handle" in
 * the callback, and the demo analyser never woke. FfiDspiCore goes with the bindings (its
 * System.loadLibrary works in one classloader only), and the contract it implements with it.
 * Both packages are pure JVM: nothing in them needs Robolectric's instrumentation.
 */
class DspiRobolectricTestRunner(testClass: Class<*>) : RobolectricTestRunner(testClass) {
    override fun createClassLoaderConfig(method: FrameworkMethod): InstrumentationConfiguration =
        InstrumentationConfiguration.Builder(super.createClassLoaderConfig(method))
            .apply { SHARED_PACKAGES.forEach { doNotAcquirePackage(it) } }
            .build()

    companion object {
        /** :ffi (FfiDspiCore, and the bindings of both namespaces under `.uniffi`) and :core-api. */
        val SHARED_PACKAGES = listOf("se.bofr.dspi.ffi.", "se.bofr.dspi.core.")
    }
}
