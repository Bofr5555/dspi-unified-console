// WP AN18: the host's HTTP GET for CG6's AutoEQ update (`autoeq_update`): the
// core carries no HTTP client, checks what comes back and writes the file;
// this only fetches. HTTPS only, bounded in time and size.
package se.bofr.dspi.feature.presets.autoeq

import java.io.IOException
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL
import se.bofr.dspi.core.AutoEqFetch
import se.bofr.dspi.core.AutoEqFetchRequest
import se.bofr.dspi.core.AutoEqFetchResponse
import se.bofr.dspi.core.DspiException

/** [AutoEqFetch] over `HttpURLConnection`. Blocking: the core calls it off the main thread. */
class HttpAutoEqFetch(
    private val open: (URL) -> HttpURLConnection = { it.openConnection() as HttpURLConnection },
    private val maxBytes: Int = MAX_BODY_BYTES
) : AutoEqFetch {
    override fun get(request: AutoEqFetchRequest): AutoEqFetchResponse {
        val url = parse(request.url) ?: throw DspiException.Transfer("only https:// addresses are fetched")
        val c = try {
            open(url)
        } catch (e: IOException) {
            throw DspiException.Transfer(e.message ?: "could not connect")
        }
        try {
            c.connectTimeout = TIMEOUT_MS
            c.readTimeout = TIMEOUT_MS
            c.instanceFollowRedirects = true
            c.setRequestProperty("User-Agent", request.userAgent)
            request.accept?.let { c.setRequestProperty("Accept", it) }
            val status = c.responseCode
            // The redirect may not leave HTTPS (HttpURLConnection never crosses protocols, checked anyway)
            if (c.url.protocol != "https") throw DspiException.Transfer("redirected away from https")
            val body = (if (status >= 400) c.errorStream else c.inputStream)?.use { read(it) } ?: ByteArray(0)
            return AutoEqFetchResponse(
                status = status,
                body = UnsignedBytes(body),
                retryAfterS = c.getHeaderField("Retry-After")?.trim()?.toLongOrNull(),
                rateLimitRemaining = c.getHeaderField("X-RateLimit-Remaining")?.trim()?.toLongOrNull(),
                rateLimitReset = c.getHeaderField("X-RateLimit-Reset")?.trim()?.toLongOrNull()
            )
        } catch (e: IOException) {
            throw DspiException.Transfer(e.message ?: "no answer")
        } finally {
            c.disconnect()
        }
    }

    private fun read(input: InputStream): ByteArray {
        val out = java.io.ByteArrayOutputStream()
        val chunk = ByteArray(64 * 1024)
        while (true) {
            val n = input.read(chunk)
            if (n < 0) break
            if (out.size() + n > maxBytes) throw IOException("the answer is too large to be a database")
            out.write(chunk, 0, n)
        }
        return out.toByteArray()
    }

    /** The body as the core's `List<Int>` of bytes, without boxing a copy of it. */
    private class UnsignedBytes(private val bytes: ByteArray) : AbstractList<Int>() {
        override val size: Int get() = bytes.size

        override fun get(index: Int): Int = bytes[index].toInt() and 0xFF
    }

    companion object {
        /** The bundled database is 5.4 MB; anything far past that is not one. */
        const val MAX_BODY_BYTES = 64 * 1024 * 1024
        const val TIMEOUT_MS = 30_000

        /** An `https://` address with a host, or null. */
        fun parse(address: String): URL? = runCatching { URL(address.trim()) }.getOrNull()
            ?.takeIf { it.protocol == "https" && !it.host.isNullOrEmpty() }
    }
}
