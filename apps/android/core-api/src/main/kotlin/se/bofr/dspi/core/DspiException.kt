package se.bofr.dspi.core

/**
 * Domain mirror of dspi-core's `DspiError` (Rust) / the UniFFI `DspiException`.
 * The adapter's generated `when` has one arm per variant of the Rust enum, so a
 * variant added there fails the :ffi build until it has a class here.
 */
sealed class DspiException(message: String) : Exception(message) {
    /** No device open, or it went away. */
    class Disconnected : DspiException("device not connected")

    /** A USB transfer failed (timeout, stall, short read, flash blackout). */
    class Transfer(val msg: String) : DspiException("USB transfer failed: $msg")

    /**
     * The device answered but refused. [status] is the raw code: `PRESET_*`,
     * `PIN_CONFIG_*`, `FLASH_*` (see [DspiConstants], [PresetStatus],
     * [PinConfigStatus], [FlashStatus]) or command-specific
     * (`testDacMute`: 3 = feature disabled).
     */
    class Refused(val status: Int) : DspiException("device refused the request (status $status)")

    class InvalidArgument(val msg: String) : DspiException("invalid argument: $msg")

    /** The connected firmware (or transport) lacks the feature. */
    class Unsupported : DspiException("not supported by the connected firmware")

    /**
     * The device's wire format is not this build's: its state is shown
     * read-only and nothing may be written.
     */
    class ReadOnly : DspiException("the connected firmware is read-only in this version")

    /**
     * DSPi Link: another console holds the device lock (macOS
     * `DeviceLock.lockedMessage`, "Another console is applying changes").
     * Nothing was sent.
     */
    class Locked : DspiException(LOCKED_MESSAGE)

    /** DSPi Link: this session's role on the hub does not allow the command. */
    class Denied : DspiException("this console's role on the hub does not allow that")

    /**
     * DSPi Link session failure: [code] is `wrong_pin`, `wrong_hub`,
     * `rate_limited`, `hub_busy`, `handshake_refused`, `io`, `handshake`,
     * `protocol`, `timeout`, `closed`, or another spec 7.1 error code.
     */
    class Link(val code: String, val msg: String) : DspiException("DSPi Link: $msg")

    /** A file could not be read or written (presets, firmware images). */
    class Io(val msg: String) : DspiException(msg)

    /**
     * An AutoEQ database update failed. [code]: `rate_limited`, `http_status`,
     * `network`, `bad_content`, `no_profiles`, `too_many_failures`, `cancelled`,
     * `stopped` or `missing`. [retryAfterS]: with `rate_limited`, seconds until
     * it is worth trying again, when the server said.
     */
    class AutoEq(val code: String, val msg: String, val retryAfterS: Long?) : DspiException(msg)

    /** The firmware guard stopped an image that can leave the board at too low a core voltage; nothing was written. */
    class FirmwareGuardBlocked(val msg: String) : DspiException(msg)

    /**
     * WP CG14: this raises the core voltage above 1.20 V from what the unit
     * reports now, and the word `OVERVOLT` was not given. Nothing was written.
     */
    class OvervoltWordRequired(val msg: String) : DspiException(msg)

    /**
     * WP P1TK: a dangerous call was not let through: confirmation tokens are
     * required and none was given, or the one given is not good for it. Nothing
     * was sent. [code]: `missing`, `unknown`, `used`, `wrong_op`, `expired` or
     * `connection_changed`.
     */
    class ConfirmationRequired(val code: String, val msg: String) : DspiException(msg)

    companion object {
        /** macOS `DeviceLock.lockedMessage`. */
        const val LOCKED_MESSAGE = "Another console is applying changes"
    }
}
