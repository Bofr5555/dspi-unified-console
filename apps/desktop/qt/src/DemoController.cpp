#include "DemoController.h"

#include "DSPiBridge.h"
#include "LinkController.h"
#include "Registrar.h"
#include "dspi_core.h"

#include <QDebug>
#include <QTimer>

static const Registrar::Controller<DemoController> demoRegistered("demo");
// Above the other banners: while it shows, nothing else here is about a real device
static const Registrar::Banner demoBanner("qrc:/qml/components/DemoBanner.qml", -10);

// FakeDevice::demo's serial, as in DSPiBridge.cpp
static const char *const kDemoSerial = "DEMO0000DSPI0001";

DemoController::DemoController(DSPiBridge *bridge, QObject *parent)
    : QObject(parent), m_bridge(bridge)
{
    connect(bridge, &DSPiBridge::stateChanged, this, &DemoController::update);
    connect(bridge, &DSPiBridge::statusChanged, this, &DemoController::update);
    connect(bridge, &DSPiBridge::devicesChanged, this, &DemoController::update);
    update();
    // The registrar makes controllers in no set order: the Link controller is there
    // once they all are
    QTimer::singleShot(0, this, [this]() {
        if (!this->parent()) return;
        m_link = this->parent()->findChild<LinkController *>(QString(), Qt::FindDirectChildrenOnly);
        if (m_link) connect(m_link.data(), &LinkController::statusChanged, this, &DemoController::update);
        update();
    });
}

// The rule LinkController gives the bridge as setHubSession
bool DemoController::hubSessionUp() const {
    if (!m_link) return false;
    const QString phase = m_link->phase();
    return phase != QLatin1String("idle") && phase != QLatin1String("closed") && phase != QLatin1String("refused");
}

void DemoController::update() {
    const bool connected = m_bridge->connected();
    const bool active = connected && m_bridge->selectedSerial() == QLatin1String(kDemoSerial);
    const bool offered = !connected && !hubSessionUp();
    const bool leavable = active && !m_bridge->availableSerials().contains(QLatin1String(kDemoSerial));
    if (active == m_active && offered == m_offered && leavable == m_leavable) return;
    m_active = active;
    m_offered = offered;
    m_leavable = leavable;
    emit changed();
}

bool DemoController::enter() {
    if (m_active) return true;
    if (hubSessionUp()) return false;
    // Closed through the bridge, so what is queued for that device is dropped first
    if (m_bridge->connected()) m_bridge->disconnectDevice();
    FfiCore *core = m_bridge->core();
    if (!dspi_open_demo(core) || !dspi_fetch_all(core)) {
        dspi_disconnect(core);
        qWarning() << "DSPi: could not open the demo device";
        m_failure = QStringLiteral("The demo could not be opened.");
        emit changed();
        return false;
    }
    m_failure.clear();
    dspi_capture_baseline(core);
    m_bridge->adoptDevice(QString::fromLatin1(kDemoSerial));   // its signals run update()
    return true;
}

bool DemoController::leave() {
    if (!m_leavable) return false;
    m_bridge->disconnectDevice();
    return true;
}
