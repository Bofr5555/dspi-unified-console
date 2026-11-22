// -- WP MDQT2c -- The auto-tune over the open devices (TuneController.h).
#include "TuneController.h"

#include "DeviceModel.h"
#include "MeasureController.h"

static const Registrar::Controller<TuneController> tuneController;

TuneController::TuneController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    // The other controllers exist once every constructor has run
    QMetaObject::invokeMethod(this, [this] {
        auto *measure = Registrar::instance<MeasureController>();
        if (!measure) return;
        for (MeasureSession *s : measure->sessions()) follow(s);
        connect(measure, &MeasureController::sessionMade, this, &TuneController::follow);
        connect(m_ctx.device, &DeviceModel::snapshotApplied, this, &TuneController::onSwitched);
        onSwitched();
    }, Qt::QueuedConnection);
}

TuneController *TuneController::create(QQmlEngine *, QJSEngine *) { return Registrar::instance<TuneController>(); }

TuneSession *TuneController::session() const { return Registrar::instance<MeasureController>()->session()->tune(); }

void TuneController::follow(MeasureSession *measure) {
    const auto relay = [this, measure](auto signal, void (TuneController::*to)()) {
        connect(measure->tune(), signal, this, [this, measure, to] {
            if (measure->isActive()) emit (this->*to)();
        });
    };
    relay(&TuneSession::stateChanged, &TuneController::stateChanged);
    relay(&TuneSession::layoutChanged, &TuneController::layoutChanged);
    relay(&TuneSession::optionsChanged, &TuneController::optionsChanged);
    relay(&TuneSession::proposalChanged, &TuneController::proposalChanged);
    relay(&TuneSession::keepConfirmChanged, &TuneController::keepConfirmChanged);
    relay(&TuneSession::abChanged, &TuneController::abChanged);
    relay(&TuneSession::errorChanged, &TuneController::errorChanged);
    relay(&TuneSession::busyChanged, &TuneController::busyChanged);
    connect(measure->tune(), &TuneSession::stepDone, this, [this, measure](const QString &step, bool ok) {
        if (measure->isActive()) emit stepDone(step, ok);
    });
}

// Another device is the active one: every property is its
void TuneController::onSwitched() {
    TuneSession *now = session();
    if (now == m_announced) return;
    m_announced = now;
    emit stateChanged();
    emit layoutChanged();
    emit optionsChanged();
    emit proposalChanged();
    emit keepConfirmChanged();
    emit abChanged();
    emit errorChanged();
    emit busyChanged();
}
