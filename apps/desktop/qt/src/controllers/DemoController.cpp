#include "DemoController.h"

#include "DeviceModel.h"
#include "DevicesController.h"
#include "LinkController.h"

#include <QJSEngine>

static const Registrar::Controller<DemoController> demoController;
static const Registrar::Banner demoBanner("DemoBanner", 35);

DemoController::DemoController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &DemoController::changed);
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

void DemoController::openDemo() {
    if (DevicesController *devices = Registrar::instance<DevicesController>()) devices->chooseDemo();
    else m_ctx.device->openDemo();
}
