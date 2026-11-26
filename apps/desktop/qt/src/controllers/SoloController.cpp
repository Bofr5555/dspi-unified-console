#include "SoloController.h"

#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"

#include <QJSEngine>

static const Registrar::Controller<SoloController> soloController;
static const Registrar::Banner soloBanner("SoloBanner", 6, Registrar::Blocking);

SoloController::SoloController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    // After every snapshot (a write, a load, a save, another device made
    // active, a disconnect): the core says whether the solo still holds
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &SoloController::read);
}

SoloController *SoloController::create(QQmlEngine *, QJSEngine *) {
    SoloController *self = Registrar::instance<SoloController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

bool SoloController::available() const { return m_ctx.device->connected() && !m_ctx.device->readOnly(); }

void SoloController::solo(int output) {
    if (!available() || output < 0 || output >= m_ctx.device->numOutputs()) return;
    const auto o = uint8_t(output);
    m_ctx.device->write(QStringLiteral("solo"), [o](FfiCore *c) { return dspi_solo_output(c, o); });
}

void SoloController::end() {
    if (m_output < 0) return;
    m_ctx.device->write(QStringLiteral("solo"), [](FfiCore *c) { return dspi_solo_end(c); });
}

void SoloController::toggle(int output) {
    if (output >= 0 && output == m_output) end();
    else solo(output);
}

// No I/O in the core, but its lock: on the worker, as every core call
void SoloController::read() {
    if (!m_ctx.device->connected()) {
        set(-1);
        return;
    }
    const DeviceSession *asked = m_ctx.activeSession();
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return dspi_solo_state(c); },
        [this, asked](int o) {
            if (m_ctx.activeSession() == asked) set(o);
        });
}

void SoloController::set(int output) {
    const bool was = available();
    if (output == m_output && was == m_available) return;
    m_output = output;
    m_available = was;
    emit changed();
}
