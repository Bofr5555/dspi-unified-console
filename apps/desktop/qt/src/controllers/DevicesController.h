#ifndef DEVICESCONTROLLER_H
#define DEVICESCONTROLLER_H

#include <QObject>
#include <QPointer>
#include <QQuickItem>
#include <QString>
#include <QThreadPool>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>
#include <atomic>

#include "CoreApi.h"

struct AppContext;
class Core;
class DeviceModel;
class DeviceRegistry;
class LinkController;
class QQmlEngine;
class QJSEngine;
class QQuickWindow;

// The device list (Device > Devices...): one list of the DSPis on USB, the demo,
// the hubs found on the network or typed in, and each hub's devices; choosing
// one opens it. QML sees it as the singleton `DevicesController`; the list and
// the pairing prompt are `DevicesHost`'s, made in the main window once it exists.
//
// Finding hubs is the core's (dspi_link_browse_*: the system's DNS-SD on
// Windows, the embedded responder on Linux, the removal grace and the .local
// address in the core); this lists what it finds and dials the URL it gives.
// It browses while the list shows and while a session waits to reconnect, so a
// hub that moved is followed (dspi_link_browse_reresolve).
class DevicesController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(bool listShown READ listShown WRITE setListShown NOTIFY listShownChanged)
    // [{serial}] from the last USB scan
    Q_PROPERTY(QVariantList usbDevices READ usbDevices NOTIFY usbDevicesChanged)
    // Look for hubs on this network (stored as hub/lookForHubs, on by default)
    Q_PROPERTY(bool lookForHubs READ lookForHubs WRITE setLookForHubs NOTIFY lookForHubsChanged)
    Q_PROPERTY(bool browsing READ browsing NOTIFY hubsChanged)
    Q_PROPERTY(QString browseError READ browseError NOTIFY hubsChanged)
    // [{id, name, kind, auth, deviceCount, serials, host, port, path, url, addresses}]
    Q_PROPERTY(QVariantList hubs READ hubs NOTIFY hubsChanged)

public:
    DevicesController(const AppContext &ctx, QObject *parent);
    ~DevicesController() override;
    static DevicesController *create(QQmlEngine *, QJSEngine *);

    bool listShown() const { return m_listShown; }
    void setListShown(bool shown);
    QVariantList usbDevices() const { return m_usb; }
    bool lookForHubs() const { return m_lookForHubs; }
    void setLookForHubs(bool look);
    bool browsing() const { return m_browsing; }
    QString browseError() const { return m_browseError; }
    QVariantList hubs() const { return m_hubs; }

    Q_INVOKABLE void openList() { setListShown(true); }
    Q_INVOKABLE void closeList() { setListShown(false); }
    // Lists the DSPis on USB again; opens nothing
    Q_INVOKABLE void rescan();
    // A local device: the hub's session is left first. -- WP MDQT1 -- It replaces
    // the active device, or with DeviceRegistry.keepOpen opens beside it. One
    // that is open already becomes the active one; open by another way (a
    // hub's unit that is on USB too), it is refused with a notice: one session
    // per device.
    Q_INVOKABLE void chooseDemo();
    Q_INVOKABLE void chooseUsb(const QString &serial);
    // A hub from `hubs`: dialled at its URL and held to its id
    Q_INVOKABLE void connectFound(const QVariantMap &hub);
    Q_INVOKABLE bool connectAddress(const QString &address);
    Q_INVOKABLE void chooseHubDevice(int handle, const QString &serial);
    // -- WP MDQT1 -- The "Open devices" list: make one active (it stays open, so
    // nothing is asked), or close it (the active one's unsaved changes are)
    Q_INVOKABLE void chooseOpen(const QString &id);
    Q_INVOKABLE void closeOpen(const QString &id);

    // This app's own hub (sharing), which is not listed as another one
    void setOwnHubId(const QString &id);
    // -- WP QP14 -- Settings > Networking shows its Other Hubs: browse while it
    // does, as while the list shows (behaviour.md §10)
    Q_INVOKABLE void setPageWatching(bool on);
    // Browsing now (the list shows or a session reconnects), for tests
    bool browseRunning() const { return m_browseRunning; }
    // The main window's host of the list and the prompt, for the smoke driver
    QQuickItem *host() const { return m_host; }

signals:
    void listShownChanged();
    void usbDevicesChanged();
    void lookForHubsChanged();
    void hubsChanged();

private:
    static void browseThunk(void *self);
    LinkController *link();
    void wire(QQuickWindow *window);
    void updateBrowse();
    void readHubs();
    void shutdown();
    // -- WP MDQT1 --
    void browseActive();
    bool takeOpen(const QString &serial, bool remote);
    void refuse(const QString &error);

    FfiCore *m_core;   // the active device's: the browse runs on it
    DeviceRegistry *m_registry;   // -- WP MDQT1 --
    Core *m_coreObject;
    DeviceModel *m_device;
    QQmlEngine *m_engine;
    QPointer<QQuickItem> m_host;
    QPointer<LinkController> m_link;
    bool m_listShown = false;
    bool m_pageWatching = false;   // -- WP QP14 --
    QVariantList m_usb;
    bool m_lookForHubs = true;
    bool m_browseRunning = false;
    bool m_browsing = false;
    QString m_browseError;
    QVariantList m_hubs;
    QByteArray m_ownHubId;
    std::atomic<bool> m_readQueued{ false };
    bool m_shutDown = false;
    QThreadPool m_reader;   // one thread: the browser's list, re-resolves
};

#endif // DEVICESCONTROLLER_H
