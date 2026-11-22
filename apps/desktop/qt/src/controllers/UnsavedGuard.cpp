// -- WP QP15 -- The unsaved-changes question on every path that leaves the
// live state (M42), settled first (SWF7)
#include "UnsavedGuard.h"
#include "Confirm.h"   // -- WP P1QT2 --

#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"   // -- WP MDQT2b --
#include "DeviceSession.h"
#include "DevicesController.h"   // -- WP LAY7 --
#include "HardwareController.h"   // -- WP QP16 --
#include "NoticeCenter.h"   // -- WP LAY7 --
#include "PresetModel.h"
#include "SessionModel.h"   // -- WP MDQT2b --
#include "Shell.h"
#include "SidebarController.h"
#include "Snapshot.h"

#include "CoreApi.h"

#include <QCoreApplication>
#include <QEvent>
#include <QGuiApplication>
#include <QJSEngine>
#include <QJsonArray>
#include <QJsonDocument>
#include <QQmlApplicationEngine>
#include <QQuickWindow>
#include <QTimer>
#include <memory>

static const Registrar::Controller<UnsavedGuard> unsavedGuard;

namespace {
// An edit's echo reaches the core's list within this long (DSPViewModel's settle)
constexpr qint64 kSettleMs = 100;
constexpr int kSummaryLines = 15;
// The status the core gives a save another console's lock refused
constexpr uint8_t kPresetLocked = 0xFE;

// The core's dirty flag and its lines. Worker thread.
std::pair<bool, QStringList> readChanges(FfiCore *c) {
    QStringList lines;
    const QJsonArray list =
        QJsonDocument::fromJson(readSized([c](char *b, uint32_t n) { return dspi_preset_changes(c, b, n); })).array();
    for (const QJsonValue &v : list) lines.append(v.toString());
    return { dspi_preset_dirty(c), lines };
}

QString failure(uint8_t status) {
    return status == kPresetLocked ? UnsavedGuard::tr("Another console is applying changes")
                                   : UnsavedGuard::tr("error %1").arg(status);
}
} // namespace

UnsavedGuard::UnsavedGuard(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    m_sinceChange.start();
    connect(ctx.device, &DeviceModel::stateChanged, this, [this](uint) { m_sinceChange.restart(); });
    ctx.shell->setQuitGuard(this, [this](std::function<void()> quit) {
        ask(QStringLiteral("quit"), this, [this, quit] {
            m_quitting = true;
            quit();
        });
    });
    if (auto *app = qobject_cast<QQmlApplicationEngine *>(ctx.engine)) {
        connect(app, &QQmlApplicationEngine::objectCreated, this, [this](QObject *object, const QUrl &) {
            if (auto *window = qobject_cast<QQuickWindow *>(object)) wire(window);
        });
    }
    // -- WP MDQT2b -- a device that goes while it is asked about (its worker
    // drops what was posted to it): on to the next one
    if (ctx.registry)
        connect(ctx.registry, &DeviceRegistry::sessionClosing, this, [this](DeviceSession *s) {
            m_targets.removeAll(s);
            if (!m_asking || s != m_target) return;
            m_target = nullptr;
            QTimer::singleShot(0, this, [this] {
                if (m_asking && !m_target) askNext();
            });
        });
}

UnsavedGuard *UnsavedGuard::create(QQmlEngine *, QJSEngine *) {
    UnsavedGuard *self = Registrar::instance<UnsavedGuard>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

void UnsavedGuard::before(const QString &op, QObject *ctx, std::function<void()> proceed) {
    UnsavedGuard *g = Registrar::instance<UnsavedGuard>();
    if (g && !op.isEmpty()) g->ask(op, ctx, std::move(proceed));
    else proceed();
}

// -- WP MDQT2b --
void UnsavedGuard::beforeClosing(DeviceSession *session, QObject *ctx, std::function<void()> proceed) {
    UnsavedGuard *g = Registrar::instance<UnsavedGuard>();
    if (g && session) g->begin(QStringLiteral("device"), { session }, ctx, std::move(proceed));
    else proceed();
}

void UnsavedGuard::setAskOnClose(bool ask) {
    if (ask == m_askOnClose) return;
    m_askOnClose = ask;
    emit askOnCloseChanged();
}

void UnsavedGuard::setAsking(bool asking) {
    if (asking == m_asking) return;
    m_asking = asking;
    emit askingChanged();
}

QString UnsavedGuard::summary(const QStringList &lines) {
    QStringList shown;
    for (int i = 0; i < lines.size() && i < kSummaryLines; ++i) shown.append(QStringLiteral("• ") + lines.at(i));
    const int more = int(lines.size()) - kSummaryLines;
    // -- WP QPI2 -- spelt out: without a .qm "%n ... (s)" shows the "(s)"
    if (more == 1) shown.append(tr("…and 1 more change"));
    else if (more > 1) shown.append(tr("…and %1 more changes").arg(more));
    return shown.join(QLatin1Char('\n'));
}

void UnsavedGuard::ask(const QString &op, QObject *ctx, std::function<void()> proceed) {
    // -- WP MDQT2b -- leaving the app asks about every open device, the
    // active one first; a device switch about the active one
    QList<DeviceSession *> targets;
    if (DeviceSession *active = m_ctx.activeSession()) targets << active;
    if (op != QLatin1String("device") && m_ctx.registry)
        for (DeviceSession *s : m_ctx.registry->sessions())
            if (!targets.contains(s)) targets << s;
    begin(op, targets, ctx, std::move(proceed));
}

void UnsavedGuard::begin(const QString &op, QList<DeviceSession *> targets, QObject *ctx, std::function<void()> proceed) {
    // One question at a time, as the Mac's modal alert allows
    if (m_asking) {
        refuseSecond(op);   // -- WP LAY7 --
        return;
    }
    m_targets.clear();
    for (DeviceSession *s : targets)
        if (s && s->model()->connected()) m_targets << s;
    // -- WP LAY7 -- leaving the app breaks off what pins a device
    const QStringList busy = op == QLatin1String("device") ? QStringList() : busyTexts();
    if (m_targets.isEmpty() && busy.isEmpty()) {
        proceed();
        return;
    }
    m_op = op;
    m_ctxObject = ctx;
    m_proceed = std::move(proceed);
    setAsking(true);
    if (busy.isEmpty()) {
        askNext();
        return;
    }
    bringBack();
    m_shown = true;
    emit busyQuestion(tr("A Device Is Busy"), busy.join(QLatin1Char('\n')) + QStringLiteral("\n\n") + tr("Quitting now breaks it off. Quit anyway?"));
}

// -- WP LAY7 --
QStringList UnsavedGuard::busyTexts() const {
    QStringList texts;
    const auto *devices = Registrar::instance<DevicesController>();
    if (!devices || !m_ctx.registry) return texts;
    for (DeviceSession *s : m_ctx.registry->sessions()) {
        const QString text = devices->pinnedBy(s);
        if (!text.isEmpty()) texts << text;
    }
    return texts;
}

void UnsavedGuard::answerBusy(const QString &answer) {
    if (!m_asking) return;
    m_shown = false;
    if (answer == QLatin1String("accept")) askNext();
    else finish();
}

// A path asking while another one's question is open: it is dropped, and the
// person hears why. The same path again before its question shows (a double
// click) says nothing: the question is on its way.
void UnsavedGuard::refuseSecond(const QString &op) {
    if (!m_shown && op == m_op) return;
    if (m_shown && m_window) {
        bringBack();
        m_window->raise();
        m_window->requestActivate();
    }
    if (auto *notices = Registrar::instance<NoticeCenter>())
        notices->post(m_shown ? tr("Answer the open question first.")
                              : tr("Still checking for unsaved changes. Try again in a moment."));
}

// The next device with something unsaved, or on with the path when none is left
void UnsavedGuard::askNext() {
    m_target = nullptr;
    while (!m_targets.isEmpty() && !m_target) {
        DeviceSession *s = m_targets.takeFirst();
        if (s && s->model()->connected()) m_target = s;
    }
    if (!m_target) {
        proceed();
        return;
    }
    int open = 0;
    if (m_ctx.registry)
        for (DeviceSession *s : m_ctx.registry->sessions())
            if (s->model()->connected()) ++open;
    m_title = open > 1 ? m_target->title() : QString();
    emit askingChanged();
    // -- WP QP16 -- Settings' unsaved changes first, as the Mac asks (F8);
    // -- WP MDQT2b -- the ones held for this device
    const HardwareController *hardware = Registrar::instance<HardwareController>();
    if (m_op == QLatin1String("device") && hardware && hardware->pendingOn(m_target)) {
        bringBack();
        m_shown = true;   // -- WP LAY7 --
        emit settingsQuestion(tr("Unsaved Settings Changes"),
                              m_target == m_ctx.activeSession()
                                  ? tr("Settings has pending changes for the current device that have not been saved. "
                                       "Switching devices will discard them.")
                                  //: %1 is a device's name
                                  : tr("Settings has pending changes for %1 that have not been saved. Closing it will discard them.")
                                        .arg(m_target->title()));
        return;
    }
    settleThenRead();
}

// -- WP QP16 --
void UnsavedGuard::answerSettings(const QString &answer) {
    if (!m_asking) return;
    m_shown = false;   // -- WP LAY7 --
    if (answer == QLatin1String("accept")) settleThenRead();
    else finish();
}

// Every write queued before the question runs first (the worker keeps order);
// then what is left of the settle window after the last change
// (-- WP MDQT2b -- on the worker of the device asked about; the settle window
// is the active one's, as only it is edited)
void UnsavedGuard::settleThenRead() {
    DeviceSession *target = m_target;
    if (!target) {
        askNext();
        return;
    }
    target->worker()->call(this, [](FfiCore *) { return 0; }, [this, target](int) {
        if (target != m_target) return;
        const qint64 left = target != m_ctx.activeSession() || target->model()->remote() ? 0 : kSettleMs - m_sinceChange.elapsed();
        if (left > 0) QTimer::singleShot(int(left), this, &UnsavedGuard::read);
        else read();
    });
}

void UnsavedGuard::read() {
    DeviceSession *target = m_target;
    if (!target) {
        askNext();
        return;
    }
    target->worker()->call(this, &readChanges, [this, target](std::pair<bool, QStringList> r) {
        if (target != m_target) return;
        if (!m_target || !m_target->model()->connected() || !r.first) {
            askNext();
            return;
        }
        bringBack();
        m_shown = true;   // -- WP LAY7 --
        emit question(m_op, summary(r.second));
    });
}

void UnsavedGuard::answer(const QString &answer) {
    if (!m_asking) return;
    m_shown = false;   // -- WP LAY7 --
    if (answer == QLatin1String("alternative")) {
        askNext();   // -- WP MDQT2b -- the next device, if any
        return;
    }
    if (answer != QLatin1String("accept")) {
        finish();
        return;
    }
    // Save into the active slot; a slot without a name gets "Preset N" first,
    // so the popup stops calling it "Empty"
    // -- WP MDQT2b -- the slot and the write of the device asked about
    SessionModel *model = m_target ? m_target->model() : nullptr;
    const DspState *state = model ? model->state() : nullptr;
    const int slot = state && state->active_preset_slot < MAX_PRESETS ? state->active_preset_slot : -1;
    if (slot < 0) {
        askNext();
        return;
    }
    const QByteArray fallback = tr("Preset %1").arg(slot + 1).toUtf8();
    auto status = std::make_shared<uint8_t>(0xFF);
    const QPointer<DeviceSession> target = m_target;
    model->write(
        QStringLiteral("preset save"),
        [slot, fallback, status](FfiCore *c) {
            char name[CHANNEL_NAME_LEN] = {};
            if (!dspi_get_preset_name(c, uint8_t(slot), name, sizeof name) || name[0] == 0)
                dspi_set_preset_name(c, uint8_t(slot), fallback.constData());
            *status = Confirm::savePreset(c, uint8_t(slot));   // -- WP P1QT2 -- token
            return *status == PRESET_OK;
        },
        [this, status, target](bool ok) {
            // -- WP QPI2 -- a device gone meanwhile has nothing left to save:
            // the switch or the quit goes on
            if (ok || !target || !target->model()->connected()) {
                askNext();
                return;
            }
            const QString why = failure(*status);
            bringBack();
            if (m_op == QLatin1String("quit")) {
                m_shown = true;   // -- WP LAY7 --
                emit saveFailed(m_op, tr("Save Failed"), tr("Failed to save preset (%1). Quit anyway?").arg(why));
                return;   // still asking: answerSaveFailed() decides
            }
            emit saveFailed(m_op, tr("Save Failed"), tr("Failed to save preset (%1). Device switch aborted.").arg(why));
            finish();
        });
}

void UnsavedGuard::answerSaveFailed(const QString &answer) {
    if (!m_asking) return;
    m_shown = false;   // -- WP LAY7 --
    if (answer == QLatin1String("accept")) proceed();
    else finish();
}

void UnsavedGuard::proceed() {
    std::function<void()> run = std::move(m_proceed);
    const bool alive = !m_ctxObject.isNull();
    finish();
    if (run && alive) run();
}

void UnsavedGuard::finish() {
    m_proceed = nullptr;
    m_ctxObject = nullptr;
    m_op.clear();
    m_targets.clear();   // -- WP MDQT2b --
    m_target = nullptr;
    m_title.clear();
    m_shown = false;   // -- WP LAY7 --
    setAsking(false);
}

// A question needs the main window: one hidden in the tray (QP14's
// TrayController, when it is there) comes back first
void UnsavedGuard::bringBack() {
    if (!m_window || m_window->isVisible()) return;
    QObject *tray = nullptr;
    if (QObject *owner = parent()) {
        for (QObject *o : owner->children())
            if (qstrcmp(o->metaObject()->className(), "TrayController") == 0) tray = o;
    }
    if (!tray || !QMetaObject::invokeMethod(tray, "restore")) m_window->show();
    m_window->raise();
    m_window->requestActivate();
}

// ---- Closing the main window --------------------------------------------------

void UnsavedGuard::wire(QQuickWindow *window) {
    if (m_window) return;
    m_window = window;
    window->installEventFilter(this);
}

// Nothing can be unsaved: no device, or the sidebar's last reading says clean
// and nothing changed or is queued since
bool UnsavedGuard::cleanForSure() const {
    if (!busyTexts().isEmpty()) return false;   // -- WP LAY7 --
    // -- WP MDQT2b -- another open device is not known clean without asking it
    if (m_ctx.registry)
        for (DeviceSession *s : m_ctx.registry->sessions())
            if (s != m_ctx.activeSession() && s->model()->connected()) return false;
    if (!m_ctx.device->connected()) return true;
    const SidebarController *sidebar = Registrar::instance<SidebarController>();
    return sidebar && !sidebar->unsaved() && !m_ctx.device->busy() && m_sinceChange.elapsed() >= kSettleMs;
}

bool UnsavedGuard::eventFilter(QObject *watched, QEvent *event) {
    if (event->type() != QEvent::Close || watched != m_window) return QObject::eventFilter(watched, event);
    if (m_closePass) {
        m_closePass = false;
        return false;
    }
    // A close that only hides the window (the tray keeps the app) loses nothing
    if (m_quitting || !m_askOnClose || !QGuiApplication::quitOnLastWindowClosed() || cleanForSure()) return false;
    event->ignore();
    QPointer<QQuickWindow> window = m_window;
    ask(QStringLiteral("close"), this, [this, window] {
        if (!window) return;
        m_closePass = true;
        window->close();
    });
    return true;
}
