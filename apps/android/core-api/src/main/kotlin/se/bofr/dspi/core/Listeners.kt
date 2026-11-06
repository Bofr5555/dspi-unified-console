package se.bofr.dspi.core

/*
 * Callback interfaces (dspi-core callbacks.rs). They run on core worker threads:
 * implementations must only schedule work (post to the bridge's dispatcher) and
 * must not call back into DspiCoreApi synchronously.
 */

/** Notification packets are queued: call [DspiCoreApi.processNotifications] (≤ 1 per 30 ms). */
fun interface NotifyListener {
    fun onNotifications()
}

/** Analyser caps or snapshot changed: call rtaGetCaps / rtaGetSnapshot. */
fun interface RtaListener {
    fun onFrame()
}

/**
 * Device arrival/departure. On Android driven by USB broadcasts in `:usb`
 * (UsbSession), not by [DspiCoreApi.pollHotplug].
 */
interface DeviceEvents {
    fun onArrived(serial: String)

    fun onDeparted(serial: String)
}

/**
 * DSPi Link session events ([DspiCoreApi.linkConnect]). Runs on the Link
 * client's reader thread: schedule work only.
 */
interface LinkListener {
    /** The hub's device list changed: call [DspiCoreApi.linkDevices]. */
    fun onLinkDevicesChanged()

    /**
     * The session ended for good: revoked, a protocol error, or refused by the
     * hub. A lost connection is not an end: the core reconnects with backoff and
     * reports it through [onLinkStatusChanged]. [code] = the hub's close code
     * (4002 revoked, 4003 shutting down), -1 = none.
     */
    fun onLinkClosed(code: Int, reason: String)

    /**
     * `linkStatus().phase` changed, or a reconnect try failed. Do-nothing default
     * until the Link engine follows the core's supervisor (WP AN3).
     */
    fun onLinkStatusChanged() {}

    /**
     * The hub issued a token: store it now against [hubId]. Comes as soon as the
     * hub's answer arrives, even if `linkPair` has already timed out. Do-nothing
     * default until WP AN3; `linkPair` still returns the token.
     */
    fun onLinkTokenIssued(hubId: String, token: String) {}
}

/**
 * The host's HTTP GET for the AutoEQ update ([DspiCoreApi.autoeqUpdate]). Called on
 * core threads, up to 20 at once while rebuilding: safe to call concurrently, with
 * a timeout. Throw [DspiException.Transfer] when no answer came.
 */
fun interface AutoEqFetch {
    @Throws(DspiException::class)
    fun get(request: AutoEqFetchRequest): AutoEqFetchResponse
}
