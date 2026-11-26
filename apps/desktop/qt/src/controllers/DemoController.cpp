#include "DemoController.h"

#include "DeviceModel.h"
#include "DevicesController.h"
#include "LinkController.h"
#include "Shell.h"                // -- WP UB12 --
#include "Snapshot.h"             // -- WP UB12 --
#include "SpeakersController.h"   // -- WP UB12 --

#include <QJSEngine>

static const Registrar::Controller<DemoController> demoController;
static const Registrar::Banner demoBanner("DemoBanner", 35, Registrar::Blocking);

DemoController::DemoController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &DemoController::changed);
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &DemoController::onSnapshot);   // -- WP UB12 --
    // -- WP UB12 -- Device > Reset Demo, the palette and the banner's button
    ctx.shell->addHandler(QStringLiteral("device.resetDemo"), this, [this] { resetDemo(); });
    // -- WP QPI4 -- a Link session's start ends "nothing connected" before its
    // first snapshot; the Link controller exists once every one is made
    QMetaObject::invokeMethod(
        this,
        [this] {
            if (LinkController *link = Registrar::instance<LinkController>())
                connect(link, &LinkController::statusChanged, this, &DemoController::changed);
        },
        Qt::QueuedConnection);
}

DemoController *DemoController::create(QQmlEngine *, QJSEngine *) {
    DemoController *self = Registrar::instance<DemoController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

bool DemoController::demoOpen() const {
    return m_ctx.device->connected() && m_ctx.device->demo() && !m_ctx.device->remote();
}

bool DemoController::nothingConnected() const {
    if (m_ctx.device->connected() || m_ctx.device->remote()) return false;
    LinkController *link = Registrar::instance<LinkController>();
    return !(link && link->inSession());
}

void DemoController::resetDemo() {
    if (!demoOpen()) return;
    m_ctx.device->resetDemo();
    // After the reset on the same worker: the example's roles, kept again
    if (auto *speakers = Registrar::instance<SpeakersController>()) speakers->reload();
}

// The Speakers page reads a device's roles when another device shows; the
// demo opened again (Reconnect, Open Demo) is the same serial, and the core
// kept the example's roles for it again
void DemoController::onSnapshot() {
    const uint64_t generation = m_ctx.device->snapshot().generation;
    if (generation == m_generation) return;
    m_generation = generation;
    if (!demoOpen()) return;
    if (auto *speakers = Registrar::instance<SpeakersController>()) speakers->reload();
}

void DemoController::openDemo() {
    if (DevicesController *devices = Registrar::instance<DevicesController>()) devices->chooseDemo();
    else m_ctx.device->openDemo();
}
