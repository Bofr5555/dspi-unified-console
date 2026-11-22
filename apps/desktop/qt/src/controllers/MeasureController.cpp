// -- WP MDQT2c -- Tools > Measure… over the open devices (MeasureController.h).
#include "MeasureController.h"

#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "DeviceSession.h"
#include "NoticeCenter.h"
#include "SpeakersController.h"

#include <QCoreApplication>
#include <QThread>

static const Registrar::Controller<MeasureController> measureController;
static const Registrar::Window measureWindow("tools.measure", "MeasureWindow");

MeasureController::MeasureController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    connect(ctx.registry, &DeviceRegistry::activeLeaving, this, [this] { session()->leaving(); });
    connect(ctx.registry, &DeviceRegistry::sessionStopping, this, [this](DeviceSession *s) {
        if (MeasureSession *m = m_sessions.value(s)) m->stopForClose();
    });
    connect(ctx.registry, &DeviceRegistry::sessionClosing, this, &MeasureController::drop);
    connect(ctx.registry, &DeviceRegistry::devicesChanged, this, [this] {
        emit runningChanged();   // a title, or the number of open devices
        checkOthers();
    });
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &MeasureController::onSwitched);
    // -- WP ROLEQ -- the speaker layout changed elsewhere (Confirm, an
    // import): the rows take its roles, the chosen outputs stay
    QMetaObject::invokeMethod(
        this,
        [this] {
            if (auto *sc = Registrar::instance<SpeakersController>())
                connect(sc, &SpeakersController::storedChanged, this, [this, sc](const QString &origin) {
                    if (origin != QLatin1String("measure")) session()->takeLayout(sc->stored());
                });
        },
        Qt::QueuedConnection);
    // Before the cores go (main.cpp shuts them down after the event loop)
    connect(qApp, &QCoreApplication::aboutToQuit, this, &MeasureController::shutdown);
    session();
    m_announced = ctx.activeSession();
}

MeasureController::~MeasureController() { shutdown(); }

MeasureController *MeasureController::create(QQmlEngine *, QJSEngine *) { return Registrar::instance<MeasureController>(); }

void MeasureController::shutdown() {
    if (m_shutDown) return;
    m_shutDown = true;
    for (MeasureSession *s : std::as_const(m_sessions)) s->shutdown();
    // Those of devices closed a moment ago, their last call returning
    for (QThread *t : findChildren<QThread *>(Qt::FindDirectChildrenOnly)) {
        t->quit();
        t->wait();
    }
}

// ---- Which session ---------------------------------------------------------------------

MeasureSession *MeasureController::session() const {
    DeviceSession *device = m_ctx.activeSession();
    if (MeasureSession *s = m_sessions.value(device)) return s;
    return make(device);
}

template <class Signal>
void MeasureController::relay(MeasureSession *from, Signal signal, void (MeasureController::*to)()) {
    connect(from, signal, this, [this, from, to] {
        if (from->isActive()) emit (this->*to)();
    });
}

MeasureSession *MeasureController::make(DeviceSession *device) const {
    auto *self = const_cast<MeasureController *>(this);
    auto *s = new MeasureSession(device, self);
    m_sessions.insert(device, s);
    if (!m_input.isEmpty()) s->adoptInput(m_input);
    if (!m_calibration.isEmpty()) s->adoptCalibration(m_calibration, m_calibrationSummary);
    if (m_shutDown) s->shutdown();
    self->relay(s, &MeasureSession::deviceChanged, &MeasureController::deviceChanged);
    self->relay(s, &MeasureSession::micChanged, &MeasureController::micChanged);
    self->relay(s, &MeasureSession::setupChanged, &MeasureController::setupChanged);
    self->relay(s, &MeasureSession::preflightChanged, &MeasureController::preflightChanged);
    self->relay(s, &MeasureSession::stateChanged, &MeasureController::stateChanged);
    self->relay(s, &MeasureSession::runningChanged, &MeasureController::runningChanged);
    self->relay(s, &MeasureSession::resultChanged, &MeasureController::resultChanged);
    self->relay(s, &MeasureSession::liveChanged, &MeasureController::liveChanged);
    self->relay(s, &MeasureSession::proposalChanged, &MeasureController::proposalChanged);
    self->relay(s, &MeasureSession::targetChanged, &MeasureController::targetChanged);
    self->relay(s, &MeasureSession::documentsChanged, &MeasureController::documentsChanged);
    self->relay(s, &MeasureSession::errorChanged, &MeasureController::errorChanged);
    self->relay(s, &MeasureSession::busyChanged, &MeasureController::busyChanged);
    self->relay(s, &MeasureSession::passiveChanged, &MeasureController::passiveChanged);
    self->relay(s, &MeasureSession::deviceSwitched, &MeasureController::deviceSwitched);
    connect(s, &MeasureSession::stored, self, [self, s](const QString &op, bool ok) {
        if (s->isActive()) emit self->stored(op, ok);
    });
    connect(s, &MeasureSession::runningChanged, self, [self, s] {
        if (!s->isActive()) self->checkOthers();
        emit self->runsChanged();   // -- WP RBFIX --
    });
    connect(s, &MeasureSession::passiveChanged, self, &MeasureController::runsChanged);   // -- WP RBFIX --
    // The microphone and its calibration are the console's
    connect(s, &MeasureSession::inputChosen, self, [self, s](const QString &choice) {
        self->m_input = choice;
        for (MeasureSession *o : std::as_const(self->m_sessions))
            if (o != s) o->adoptInput(choice);
    });
    connect(s, &MeasureSession::calibrationChosen, self, [self, s] {
        self->m_calibration = s->micDocument();
        self->m_calibrationSummary = s->micSummary();
        for (MeasureSession *o : std::as_const(self->m_sessions))
            if (o != s) o->adoptCalibration(self->m_calibration, self->m_calibrationSummary);
    });
    emit self->sessionMade(s);
    return s;
}

// Another device is the active one: every property is its
void MeasureController::onSwitched() {
    DeviceSession *device = m_ctx.activeSession();
    if (device == m_announced) return;
    m_announced = device;
    session()->entered();
    emit deviceSwitched();
    emit deviceChanged();
    emit micChanged();
    emit setupChanged();
    emit preflightChanged();
    emit stateChanged();
    emit runningChanged();
    checkOthers();
    emit resultChanged();
    emit liveChanged();
    emit proposalChanged();
    emit targetChanged();
    emit documentsChanged();
    emit errorChanged();
    emit busyChanged();
    emit passiveChanged();
}

void MeasureController::drop(DeviceSession *device) {
    delete m_sessions.take(device);
    checkOthers();
    emit runsChanged();   // -- WP RBFIX --
}

// ---- What runs where -------------------------------------------------------------------

MeasureSession *MeasureController::capturing(const MeasureSession *but) const {
    for (MeasureSession *s : std::as_const(m_sessions))
        if (s != but && s->capturing()) return s;
    return nullptr;
}

void MeasureController::report(MeasureSession *on, const QString &text, bool warning) {
    if (m_shutDown || text.isEmpty()) return;
    if (auto *notices = Registrar::instance<NoticeCenter>())
        //: %1 is a device, %2 what happened on it
        notices->post(tr("%1: %2").arg(on->session()->title(), text), warning ? QStringLiteral("warning") : QStringLiteral("info"));
}

QString MeasureController::runningOn() const {
    return m_ctx.registry->devices().size() > 1 ? m_ctx.activeSession()->title() : QString();
}

// A run's state comes several times a second: the rows (and their Stop buttons)
// are made anew only when they say something else
void MeasureController::checkOthers() {
    const QVariantList now = others();
    if (now == m_others) return;
    m_others = now;
    emit otherRunsChanged();
}

QVariantList MeasureController::others() const {
    QVariantList out;
    for (DeviceSession *d : m_ctx.registry->sessions()) {
        MeasureSession *s = m_sessions.value(d);
        if (!s || s->isActive() || !s->running()) continue;
        out.append(QVariantMap{ { QStringLiteral("id"), d->id() },
                                { QStringLiteral("title"), d->title() },
                                { QStringLiteral("text"), s->runningText() } });
    }
    return out;
}

MeasureSession *MeasureController::byId(const QString &id) const { return m_sessions.value(m_ctx.registry->session(id)); }

void MeasureController::stopOther(const QString &id) {
    if (MeasureSession *s = byId(id)) s->stop();
}

void MeasureController::showOther(const QString &id) { m_ctx.registry->activate(id); }

void MeasureController::stopAll() {
    for (MeasureSession *s : std::as_const(m_sessions))
        if (s->isActive() || s->underWay()) s->stop();
}

// -- WP RBFIX --
QString MeasureController::runsOn() const {
    QStringList titles;
    for (DeviceSession *d : m_ctx.registry->sessions())
        if (const MeasureSession *s = m_sessions.value(d); s && s->underWay()) titles << d->title();
    return titles.join(QStringLiteral(", "));
}
