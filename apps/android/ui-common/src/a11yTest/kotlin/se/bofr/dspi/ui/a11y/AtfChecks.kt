package se.bofr.dspi.ui.a11y

import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityNodeProvider
import com.google.android.apps.common.testing.accessibility.framework.AccessibilityCheckPreset
import com.google.android.apps.common.testing.accessibility.framework.AccessibilityCheckResult.AccessibilityCheckResultType
import com.google.android.apps.common.testing.accessibility.framework.AccessibilityHierarchyCheckResult
import com.google.android.apps.common.testing.accessibility.framework.uielement.AccessibilityHierarchyAndroid
import java.util.Locale
import org.junit.Assert.assertEquals

/**
 * WP AN20: the Accessibility Test Framework's checks (the LATEST preset, as
 * Espresso's `AccessibilityValidator` runs them) over a window's
 * accessibility nodes, Compose's virtual nodes included, in Robolectric.
 *
 * Why not `enableAccessibilityChecks()`: ATF 4.1.1 builds its hierarchy from
 * plain Views when `Build.FINGERPRINT` is "robolectric", and on that path it
 * reports Compose content as "not evaluated in this environment"; the node
 * path it takes on a device asks the window for children over an in-process
 * accessibility connection, which Robolectric does not serve. So this walks
 * the Views itself, switches Compose's accessibility on (no service runs in
 * Robolectric), asks each `AccessibilityNodeProvider` (Compose's) for the
 * node of every virtual child, as TalkBack would, and hands ATF's node-info
 * builder a tree whose children are those nodes.
 *
 * Tests that use it run with `@GraphicsMode(NATIVE)`: Compose leaves out
 * nodes it finds covered with `android.graphics.Region`, which the legacy
 * graphics mode only stubs.
 *
 * Shared by the modules' tests as an extra test source directory (each
 * module's build.gradle.kts, `-- WP AN20 --`).
 */
object AtfChecks {
    /** Every result of the LATEST preset of [type] (ERROR by default) for the window [root] is in, as text. */
    fun results(root: View, type: AccessibilityCheckResultType = AccessibilityCheckResultType.ERROR): List<String> = withRealNodes {
        val hierarchy = hierarchy(root) ?: return@withRealNodes emptyList()
        val checks = AccessibilityCheckPreset.getAccessibilityHierarchyChecksForPreset(AccessibilityCheckPreset.LATEST)
        checks.flatMap { check -> check.runCheckOnHierarchy(hierarchy) }
            .filter { it.type == type }
            .map(::describe)
    }

    /** Fails with every ATF error on the window [root] is in. */
    fun assertPass(root: View, what: String) {
        assertEquals("$what: Accessibility Test Framework errors", emptyList<String>(), results(root))
    }

    /** How many nodes ATF saw: a check that saw only the window frame proves nothing. */
    fun nodeCount(root: View): Int = withRealNodes { hierarchy(root)?.activeWindow?.allViews?.size ?: 0 }

    private fun hierarchy(root: View): AccessibilityHierarchyAndroid? {
        val node = nodeFor(root.rootView) ?: return null
        return AccessibilityHierarchyAndroid.newBuilder(node, root.context).build()
    }

    private fun describe(r: AccessibilityHierarchyCheckResult): String {
        val e = r.element
        val label = e?.let { "${it.className} \"${it.contentDescription ?: it.text ?: ""}\" ${it.boundsInScreen}" } ?: "?"
        return "${r.sourceCheckClass.simpleName}: $label: ${r.getMessage(Locale.ENGLISH)}"
    }

    /**
     * Robolectric's node shadow keeps its own child list and compares nodes by their View alone (every
     * Compose node shares one); the framework's own nodes tell virtual nodes apart, as ATF needs.
     */
    private fun <T> withRealNodes(block: () -> T): T {
        val key = "robolectric.useRealAni"
        val before = System.getProperty(key)
        System.setProperty(key, "true")
        try {
            return block()
        } finally {
            if (before == null) System.clearProperty(key) else System.setProperty(key, before)
        }
    }

    private fun nodeFor(view: View): AccessibilityNodeInfo? {
        if (view.visibility != View.VISIBLE) return null
        val provider: AccessibilityNodeProvider? = view.accessibilityNodeProvider
        if (provider != null) {
            forceAccessibility(view)
            return virtualNode(provider, AccessibilityNodeProvider.HOST_VIEW_ID, depth = 0)
        }
        val kids = if (view is ViewGroup) (0 until view.childCount).mapNotNull { nodeFor(view.getChildAt(it)) } else emptyList()
        return Linked(view.createAccessibilityNodeInfo(), kids)
    }

    private fun virtualNode(provider: AccessibilityNodeProvider, id: Int, depth: Int): AccessibilityNodeInfo? {
        check(depth < 200) { "virtual node tree too deep" }
        val node = provider.createAccessibilityNodeInfo(id) ?: return null
        val kids = virtualChildIds(node).mapNotNull { virtualNode(provider, it, depth + 1) }
        return Linked(node, kids)
    }

    /**
     * Compose builds its node tree only while an accessibility service runs; Robolectric has none, so
     * switch it on as Compose's own test framework does (`AndroidComposeView` is internal: by name).
     */
    private fun forceAccessibility(view: View) {
        runCatching { view.javaClass.getMethod("forceAccessibilityForTesting", Boolean::class.javaPrimitiveType) }
            .getOrNull()
            ?.invoke(view, true)
    }

    /** The virtual ids the provider recorded with addChild(View, Int): the framework's child node ids, upper 32 bits. */
    private fun virtualChildIds(node: AccessibilityNodeInfo): List<Int> {
        val f = AccessibilityNodeInfo::class.java.getDeclaredField("mChildNodeIds").apply { isAccessible = true }
        val ids = f.get(node) ?: return emptyList()
        val size = ids.javaClass.getMethod("size").invoke(ids) as Int
        val get = ids.javaClass.getMethod("get", Int::class.javaPrimitiveType)
        return (0 until size).map { ((get.invoke(ids, it) as Long) ushr 32).toInt() }
    }

    /** A copy of [node] whose children are [kids]: what a window's accessibility connection would return. */
    private class Linked(node: AccessibilityNodeInfo, private val kids: List<AccessibilityNodeInfo>) : AccessibilityNodeInfo(node) {
        init {
            // As a node an accessibility service receives: read-only, so its queries are allowed
            AccessibilityNodeInfo::class.java.getDeclaredMethod("setSealed", Boolean::class.javaPrimitiveType).invoke(this, true)
        }

        override fun getChildCount() = kids.size

        override fun getChild(index: Int): AccessibilityNodeInfo = kids[index]

        override fun getChild(index: Int, prefetchingStrategy: Int): AccessibilityNodeInfo = kids[index]
    }
}
