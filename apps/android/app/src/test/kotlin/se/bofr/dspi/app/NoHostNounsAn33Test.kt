package se.bofr.dspi.app

import java.io.File
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * WP AN33 (owner rule): user-visible text never names the host device ("phone", "tablet",
 * "computer", "Mac", "PC") and never says "this phone". Scans every string literal in the
 * modules' main Kotlin sources, the `res/values*` string resources and the Play listing and
 * release notes. "device" stays allowed: it means the DSPi.
 */
class NoHostNounsAn33Test {
    private val root = File("..").canonicalFile // the Gradle project: apps/android

    @Test
    fun wpAn33_noUserVisibleTextNamesTheHost() {
        val hits = mutableListOf<String>()
        fun check(where: String, text: String) {
            if (text in ALLOWED) return
            val words = text.lowercase().split(Regex("[^a-z0-9']+"))
            if (words.any { it.removeSuffix("'s") in HOST }) hits += "$where: $text"
        }
        val kotlin = root.listFiles().orEmpty().filter { File(it, "src/main").isDirectory }
            .flatMap { File(it, "src/main").walkTopDown().filter { f -> f.isFile && f.extension == "kt" }.toList() }
        assertTrue("no Kotlin sources found under $root", kotlin.size > 50)
        for (f in kotlin) for (s in stringLiterals(f.readText())) check(f.relativeTo(root).path, s)
        val resources = root.walkTopDown().onEnter { it.name != "build" }
            .filter { it.isFile && it.extension == "xml" && it.parentFile.name.startsWith("values") && "/src/main/res/" in it.path }
        for (f in resources) {
            Regex(">([^<]*)<").findAll(f.readText()).forEach { check(f.relativeTo(root).path, it.groupValues[1]) }
        }
        val store = listOf("store/play/listing", "store/play/release-notes").map { File(root, it) }
        assertTrue("store texts missing", store.all { it.isDirectory })
        for (dir in store) {
            for (f in dir.walkTopDown().filter { it.isFile && it.extension == "txt" }) {
                f.readLines().forEach { check(f.relativeTo(root).path, it) }
            }
        }
        assertTrue("host-device nouns in user-visible text:\n" + hits.joinToString("\n"), hits.isEmpty())
    }

    @Test
    fun wpAn33_theScannerFindsLiteralsAndSkipsComments() {
        val src = "// this phone\n/* a phone */ val a = \"on this phone\"\n" +
            "val b = \"\"\"a \"tablet\"\"\"\"\nval c = '\"'\nval d = \"x \\\" \${y(\"z\")}\""
        val found = stringLiterals(src)
        assertTrue(found.toString(), found.contains("on this phone"))
        assertTrue(found.toString(), found.any { "tablet" in it })
        assertTrue(found.toString(), found.none { it == " this phone" || it == " a phone " })
    }

    companion object {
        private val HOST = setOf(
            "phone", "phones", "tablet", "tablets", "computer", "computers", "mac", "macs", "pc", "pcs",
            "iphone", "ipad", "laptop", "laptops", "smartphone", "handset"
        )

        /** Not the host: Psybass starting points name the speaker the bass is for (feature-tools). */
        private val ALLOWED = setOf("Laptop / tablet")

        /**
         * The contents of every string literal ("..." and """...""") in [src], comments and
         * char literals skipped. Template expressions stay inside the text: they never hold
         * user-visible words, and nested literals in them are rare enough to tolerate.
         */
        fun stringLiterals(src: String): List<String> {
            val out = mutableListOf<String>()
            var i = 0
            while (i < src.length) {
                when {
                    src.startsWith("//", i) -> i = src.indexOf('\n', i).let { if (it < 0) src.length else it }
                    src.startsWith("/*", i) -> i = src.indexOf("*/", i + 2).let { if (it < 0) src.length else it + 2 }
                    src.startsWith("\"\"\"", i) -> {
                        var end = src.indexOf("\"\"\"", i + 3).let { if (it < 0) src.length else it }
                        while (end + 3 < src.length && src[end + 3] == '"') end++ // """a"""" ends at the last three
                        out += src.substring(i + 3, end)
                        i = end + 3
                    }
                    src[i] == '"' -> {
                        val sb = StringBuilder()
                        i++
                        while (i < src.length && src[i] != '"' && src[i] != '\n') {
                            if (src[i] == '\\' && i + 1 < src.length) i++
                            sb.append(src[i])
                            i++
                        }
                        out += sb.toString()
                        i++
                    }
                    src[i] == '\'' -> {
                        i++
                        while (i < src.length && src[i] != '\'' && src[i] != '\n') i += if (src[i] == '\\') 2 else 1
                        i++
                    }
                    else -> i++
                }
            }
            return out
        }
    }
}
