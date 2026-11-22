#include "DevicesController.h"

#include "Core.h"
#include "CoreVoltageController.h"   // -- WP MDQT2b --
#include "DeviceModel.h"
#include "DeviceRegistry.h"   // -- WP MDQT1 --
#include "DeviceSession.h"
#include "FirmwareController.h"   // -- WP MDQT2b --
#include "LinkController.h"
#include "NoticeCenter.h"
#include "Registrar.h"
#include "SessionModel.h"
#include "Shell.h"
#include "Snapshot.h"
#include "UnsavedGuard.h"   // -- WP QP15 --

#include <QCoreApplication>
#include <QJSEngine>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQuickItem>
#include <QPointer>
#include <QQuickWindow>
#include <QSettings>
#include <QVector>

static const Registrar::Controller<DevicesController> devicesController;

namespace {
const char kLookForHubs[] = "hub/lookForHubs";

// The DSPis on USB, as [{serial}]. Worker thread.
QVariantList scanUsb(FfiCore *c) {
    QVector<DeviceInfo> found(16);
    const uint32_t n = dspi_scan_devices(c, found.data(), uint32_t(found.size()));
    QVariantList out;
    for (uint32_t i = 0; i < n && i < uint32_t(found.size()); ++i) {
        const DeviceInfo &d = found[int(i)];
        const uint32_t len = qMin<uint32_t>(d.serial_len, uint32_t(sizeof d.serial));
        out.append(QVariantMap{
            { QStringLiteral("serial"), QString::fromUtf8(reinterpret_cast<const char *>(d.serial), int(len)) } });
    }
    return out;
}
} // namespace

DevicesController::DevicesController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_core(ctx.core->handle()), m_registry(ctx.registry), m_coreObject(ctx.core), m_device(ctx.device),
      m_engine(ctx.engine) {
    m_reader.setMaxThreadCount(1);
    m_reader.setExpiryTimeout(-1);
    m_lookForHubs = QSettings().value(QLatin1String(kLookForHubs), true).toBool();

    ctx.shell->addHandler(QStringLiteral("device.devices"), this, [this] { openList(); });
    ctx.shell->addHandler(QStringLiteral("device.reconnect"), this, [this] {
        // A hub's session reconnects by itself; Reconnect asks it to now
        if (m_device->remote()) {
            if (LinkController *l = link()) l->retryNow();
        } else {
            m_device->reconnect();
        }
    });
    connect(ctx.core, &Core::deviceEvent, this, [this] {
        if (m_listShown) rescan();
    });
    // -- WP MDQT1 --
    connect(m_registry, &DeviceRegistry::activeChanged, this, &DevicesController::browseActive);
    connect(m_registry, &DeviceRegistry::openFailed, this, [this](const QString &, const QString &error) { refuse(error); });
    connect(m_registry, &DeviceRegistry::configFailed, this, [](const QString &error) {
        if (auto *notices = Registrar::instance<NoticeCenter>())
            //: %1 is the system's reason
            notices->post(tr("A data folder cannot be used, so what belongs in it is not kept: %1").arg(error),
                          QStringLiteral("warning"));
    });
    // The list and the prompt live in the main window, made once it exists
    if (auto *app = qobject_cast<QQmlApplicationEngine *>(ctx.engine)) {
        connect(app, &QQmlApplicationEngine::objectCreated, this, [this](QObject *object, const QUrl &) {
            if (auto *window = qobject_cast<QQuickWindow *>(object)) wire(window);
        });
    }
    connect(qApp, &QCoreApplication::aboutToQuit, this, &DevicesController::shutdown);
}

DevicesController::~DevicesController() { shutdown(); }

DevicesController *DevicesController::create(QQmlEngine *, QJSEngine *) {
    DevicesController *self = Registrar::instance<DevicesController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

LinkController *DevicesController::link() {
    if (!m_link) m_link = Registrar::instance<LinkController>();
    return m_link;
}

void DevicesController::wire(QQuickWindow *window) {
    if (m_host) return;
    if (LinkController *l = link()) {
        connect(l, &LinkController::statusChanged, this, &DevicesController::updateBrowse);
        // A hub that does not answer may be gone: the browser asks the network
        connect(l, &LinkController::dialFailed, this, [this](const QString &hubId) {
            if (hubId.isEmpty() || !m_browseRunning) return;
            const QByteArray id = hubId.toUtf8();
            m_reader.start([c = m_core, id] { dspi_link_browse_verify(c, id.constData()); });   // -- WP MDQT1 -- c
        });
    }
    QQmlComponent component(m_engine, QStringLiteral("UConsole"), QStringLiteral("DevicesHost"));
    QObject *made = component.createWithInitialProperties(
        { { QStringLiteral("parent"), QVariant::fromValue(window->contentItem()) } });
    m_host = qobject_cast<QQuickItem *>(made);
    if (!m_host) {
        qWarning("DevicesController: the device list did not load: %s", qPrintable(component.errorString()));
        delete made;
        return;
    }
    m_host->setParent(window->contentItem());
}

void DevicesController::shutdown() {
    if (m_shutDown) return;
    m_shutDown = true;
    m_reader.waitForDone();
    if (m_browseRunning) dspi_link_browse_stop(m_core);   // waits out a callback in progress
    m_browseRunning = false;
}

void DevicesController::setListShown(bool shown) {
    if (shown == m_listShown) return;
    m_listShown = shown;
    emit listShownChanged();
    if (shown) rescan();
    updateBrowse();
}

void DevicesController::setLookForHubs(bool look) {
    if (look == m_lookForHubs) return;
    m_lookForHubs = look;
    QSettings().setValue(QLatin1String(kLookForHubs), look);
    emit lookForHubsChanged();
    updateBrowse();
}

void DevicesController::setOwnHubId(const QString &id) {
    const QByteArray own = id.toUtf8();
    if (own == m_ownHubId) return;
    m_ownHubId = own;
    if (m_browseRunning) {
        m_browseRunning = false;   // started again below with the new id
        dspi_link_browse_stop(m_core);
    }
    updateBrowse();
}

// -- WP QP14 --
void DevicesController::setPageWatching(bool on) {
    if (on == m_pageWatching) return;
    m_pageWatching = on;
    updateBrowse();
}

void DevicesController::rescan() {
    m_coreObject->worker()->call(this, [](FfiCore *c) { return scanUsb(c); },
                                 [this](QVariantList found) {
                                     if (found == m_usb) return;
                                     m_usb = found;
                                     emit usbDevicesChanged();
                                 });
}

// ---- Finding hubs --------------------------------------------------------------

// The browser's thread: schedule and return
void DevicesController::browseThunk(void *self) {
    QMetaObject::invokeMethod(static_cast<DevicesController *>(self), &DevicesController::readHubs,
                              Qt::QueuedConnection);
}

void DevicesController::updateBrowse() {
    if (m_shutDown) return;
    LinkController *l = m_link;
    const bool reconnecting = l && l->phase() == QLatin1String("reconnecting");
    const bool want = m_lookForHubs && (m_listShown || reconnecting || m_pageWatching);   // -- WP QP14 -- the page
    if (want && !m_browseRunning) {
        m_browseRunning = dspi_link_browse_start(m_core, &DevicesController::browseThunk, this,
                                                 m_ownHubId.isEmpty() ? nullptr : m_ownHubId.constData());
        readHubs();
    } else if (!want && m_browseRunning) {
        m_browseRunning = false;
        dspi_link_browse_stop(m_core);
        readHubs();
    } else if (reconnecting) {
        readHubs();   // the hub may be listed at a new address already
    }
}

void DevicesController::readHubs() {
    if (m_shutDown || m_readQueued.exchange(true)) return;
    // A session waiting to reconnect follows its hub to where it is announced now
    const QByteArray follow =
        m_link && m_link->phase() == QLatin1String("reconnecting") ? m_link->hubId().toUtf8() : QByteArray();
    m_reader.start([this, follow, c = m_core] {   // -- WP MDQT1 -- c, which may be another by then
        m_readQueued = false;
        const QJsonObject o = QJsonDocument::fromJson(
                                  readSized([c](char *b, uint32_t n) { return dspi_link_browse_hubs(c, b, n); }, 4096))
                                  .object();
        if (!follow.isEmpty()) dspi_link_browse_reresolve(c, follow.constData());
        const bool browsing = o.value(QLatin1String("browsing")).toBool();
        const QString error = o.value(QLatin1String("error")).toString();
        const QVariantList hubs = o.value(QLatin1String("hubs")).toArray().toVariantList();
        QMetaObject::invokeMethod(
            this,
            [this, browsing, error, hubs] {
                if (browsing == m_browsing && error == m_browseError && hubs == m_hubs) return;
                m_browsing = browsing;
                m_browseError = error;
                m_hubs = hubs;
                emit hubsChanged();
            },
            Qt::QueuedConnection);
    });
}

// ---- Choosing ----------------------------------------------------------------

// -- WP MDQT1 --
// The browse runs on the active device's core, whose session it follows to a
// new address (dspi_link_browse_reresolve)
void DevicesController::browseActive() {
    if (m_shutDown) return;
    if (m_browseRunning) {
        m_browseRunning = false;
        dspi_link_browse_stop(m_core);
    }
    m_core = m_coreObject->handle();
    updateBrowse();
}

void DevicesController::refuse(const QString &error) {
    if (auto *notices = Registrar::instance<NoticeCenter>())
        notices->post(error.isEmpty() ? tr("The device did not open.") : error, QStringLiteral("warning"));
}

// The unit is open in a session that is not the active one: made active when
// it is reached the same way, refused when not
bool DevicesController::takeOpen(const QString &serial, bool remote) {
    DeviceSession *holder = m_registry->holder(serial);
    if (!holder || holder == m_registry->active()) return false;
    if (holder->model()->remote() == remote) {
        m_registry->activate(holder);
    } else {
        //: %1 is the device's serial number
        refuse(remote ? tr("%1 is already open over USB. Close it there first.").arg(serial)
                      : tr("%1 is already open through a hub. Close it there first.").arg(serial));
    }
    return true;
}

void DevicesController::chooseOpen(const QString &id) { m_registry->activate(id); }

void DevicesController::closeOpen(const QString &id) {
    // -- WP MDQT2b -- that device's question, and not while a job pins it
    DeviceSession *session = m_registry->session(id);
    if (!session) return;
    const QString pinned = pinnedBy(session);
    if (!pinned.isEmpty()) {
        refuse(pinned);
        return;
    }
    const QPointer<DeviceSession> asked = session;
    UnsavedGuard::beforeClosing(session, this, [this, id, asked] {
        if (!asked) return;
        const QString now = pinnedBy(asked);
        if (now.isEmpty()) m_registry->close(id);
        else refuse(now);
    });
}

// -- WP MDQT2b --
QString DevicesController::pinnedText(const QString &id) const { return pinnedBy(m_registry->session(id)); }

QString DevicesController::pinnedBy(DeviceSession *session) const {
    if (!session) return QString();
    const auto *firmware = Registrar::instance<FirmwareController>();
    //: %1 is a device's name
    if (firmware && firmware->runsOn(session)) return tr("A firmware update is under way on %1. Wait for it to finish.").arg(session->title());
    const auto *voltage = Registrar::instance<CoreVoltageController>();
    //: %1 is a device's name
    if (voltage && voltage->calibratingOn(session))
        return tr("A core voltage calibration runs on %1. End it on its Core Voltage page first.").arg(session->title());
    return QString();
}

// Classic replaces the active device: not while a job pins it
bool DevicesController::refusedPinned() {
    if (m_registry->keepOpen()) return false;
    const QString pinned = pinnedBy(m_registry->active());
    if (pinned.isEmpty()) return false;
    refuse(pinned);
    return true;
}

void DevicesController::chooseDemo() {
    // -- WP MDQT1 -- beside the open ones
    if (m_registry->keepOpen()) {
        m_registry->openDemo();
        return;
    }
    // -- WP QP15 -- another device: the unsaved-changes question first (M42, QD37)
    const bool same = m_device->demo() && !m_device->remote();
    if (!same && refusedPinned()) return;   // -- WP MDQT2b --
    UnsavedGuard::before(same ? QString() : QStringLiteral("device"), this, [this] {
        if (LinkController *l = link(); l && (l->inSession() || m_device->remote())) l->leaveForLocal();
        m_device->openDemo();
    });
}

void DevicesController::chooseUsb(const QString &serial) {
    if (serial.isEmpty() || takeOpen(serial, false)) return;   // -- WP MDQT1 --
    // -- WP MDQT1 -- beside the open ones
    if (m_registry->keepOpen() && m_registry->holder(serial) != m_registry->active()) {
        m_registry->openUsb(serial);
        return;
    }
    // -- WP QP15 -- another device: the unsaved-changes question first (M42, QD37)
    const bool same = serial == m_device->serial() && !m_device->demo() && !m_device->remote();
    if (!same && refusedPinned()) return;   // -- WP MDQT2b --
    UnsavedGuard::before(same ? QString() : QStringLiteral("device"), this, [this, serial] {
        if (LinkController *l = link(); l && (l->inSession() || m_device->remote())) l->leaveForLocal();
        m_device->selectDevice(serial);
    });
}

void DevicesController::connectFound(const QVariantMap &hub) {
    const QString url = hub.value(QStringLiteral("url")).toString();
    if (url.isEmpty()) return;
    // -- WP SECQT -- what the hub's record advertises: TLS then never falls
    // back to plaintext (ratchet (a))
    if (LinkController *l = link())
        l->connectToHub(url, hub.value(QStringLiteral("id")).toString(), hub.value(QStringLiteral("tlsAdvertised")).toBool());
}

bool DevicesController::connectAddress(const QString &address) {
    LinkController *l = link();
    return l && l->connectToAddress(address);
}

void DevicesController::chooseHubDevice(int handle, const QString &serial) {
    if (takeOpen(serial, true)) return;   // -- WP MDQT1 --
    // -- WP MDQT1 -- beside the open ones
    if (m_registry->keepOpen() && m_registry->holder(serial) != m_registry->active()) {
        if (LinkController *l = link()) l->bindDevice(handle, serial);
        return;
    }
    // -- WP QP15 -- another device: the unsaved-changes question first (M42)
    const bool same = serial == m_device->serial() && m_device->remote();
    if (!same && refusedPinned()) return;   // -- WP MDQT2b --
    UnsavedGuard::before(same ? QString() : QStringLiteral("device"), this, [this, handle, serial] {
        if (LinkController *l = link()) l->bindDevice(handle, serial);
    });
}
