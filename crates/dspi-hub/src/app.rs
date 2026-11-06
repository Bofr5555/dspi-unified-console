//! The embedding app as the hub's local session (HUB.md section 4).
//!
//! While an [`AppLink`] exists, a [`DspiCore`] gets its USB devices from the hub:
//! `select_device`, `scan_devices` and `poll_hotplug` go through the broker, and the
//! rest of the core does not know. The core calls the broker with its lock released,
//! and no hub thread ever calls the core, so the core's lock can wait on a hub worker
//! and never the reverse.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use dspi_core::{BrokeredDevice, DeviceBroker, DeviceInfo, DspiCore, DspiError};

use crate::error::HubError;
use crate::hub::{Hub, HubHandle};
use crate::registry::DeviceState;

struct Broker(HubHandle);

impl DeviceBroker for Broker {
    fn devices(&self) -> Vec<DeviceInfo> {
        let mut found: Vec<DeviceInfo> = self
            .0
            .devices()
            .into_iter()
            .filter(|d| d.state == DeviceState::Online)
            .map(|d| DeviceInfo {
                serial: d.identity.serial,
                location_id: d.identity.location_id,
            })
            .collect();
        found.sort_by(|a, b| a.serial.cmp(&b.serial));
        found
    }

    fn inaccessible(&self) -> u32 {
        self.0.inaccessible()
    }

    fn open(&self, serial: &str) -> dspi_core::Result<BrokeredDevice> {
        let vid = self
            .0
            .devices()
            .iter()
            .find(|d| d.identity.serial == serial)
            .map_or(0, |d| d.identity.vid);
        match self.0.open_local(serial) {
            Ok(transport) => Ok(BrokeredDevice {
                transport: Box::new(transport),
                vid,
            }),
            Err(e) => {
                log::debug!("hub: {serial} not opened: {e}");
                Err(DspiError::Disconnected)
            }
        }
    }
}

/// A core attached to a hub. Dropping it detaches the core, as [`AppLink::detach`].
pub struct AppLink {
    hub: HubHandle,
    core: Arc<DspiCore>,
    active: bool,
}

impl AppLink {
    /// Route `core`'s USB devices through `hub`.
    ///
    /// A device the core holds directly is closed, the hub claims it, and the core
    /// opens it again through the hub, so the mirror is read afresh. Blocks for all
    /// that: off the UI thread. A flow in progress on the old connection fails with
    /// `Disconnected`, and a running analyser must be started again, as after any
    /// reconnect. A device that does not come back is left to `poll_hotplug`.
    ///
    /// [`HubError::AppAttached`] if this hub already has an app: it has one local
    /// session, and two apps would each take the other's writes for their own.
    pub fn attach(hub: &HubHandle, core: Arc<DspiCore>) -> Result<Self, HubError> {
        if hub.is_stopped() {
            return Err(HubError::Stopped);
        }
        if hub.inner.app_attached.swap(true, Ordering::SeqCst) {
            return Err(HubError::AppAttached);
        }
        let was_open = core.set_device_broker(Some(Arc::new(Broker(hub.clone()))));
        // The device the core just let go of; the scan at the hub's start found it taken.
        hub.scan_now();
        if let Some(serial) = was_open
            && let Err(e) = core.select_device(serial.clone())
        {
            log::warn!("reopening {serial} through the hub failed: {e}");
        }
        Ok(Self {
            hub: hub.clone(),
            core,
            active: true,
        })
    }

    /// Give the core its direct USB path back. Closes a device it holds through the
    /// hub and returns its serial, to open again once the hub has let go of it.
    pub fn detach(mut self) -> Option<String> {
        self.release()
    }

    /// The whole way back: detach, stop the hub, then reopen the device directly. The
    /// order matters: the core cannot claim a USB interface until the hub has dropped
    /// it, which [`Hub::stop`] guarantees on return. Blocks: off the UI thread.
    pub fn stop_hub(mut self, hub: &Hub) {
        let was_open = self.release();
        hub.stop();
        if let Some(serial) = was_open
            && let Err(e) = self.core.select_device(serial.clone())
        {
            log::info!("{serial} not reopened after the hub stopped: {e}");
        }
    }

    fn release(&mut self) -> Option<String> {
        if !std::mem::take(&mut self.active) {
            return None;
        }
        let was_open = self.core.set_device_broker(None);
        self.hub.inner.app_attached.store(false, Ordering::SeqCst);
        was_open
    }
}

impl Drop for AppLink {
    fn drop(&mut self) {
        self.release();
    }
}
