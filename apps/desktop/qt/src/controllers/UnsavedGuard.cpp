// -- WP QP15 -- The unsaved-changes question on every path that leaves the
// live state (M42), settled first (SWF7)
#include "UnsavedGuard.h"

#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "PresetModel.h"
#include "Shell.h"
#include "SidebarController.h"
#include "Snapshot.h"

#include "CoreApi.h"

#include <QCoreApplication>
#include <QEvent>
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
    if (more > 0) shown.append(tr("...and %n more change(s)", nullptr, more));
    return shown.join(QLatin1Char('\n'));
}

void UnsavedGuard::ask(const QString &op, QObject *ctx, std::function<void()> proceed) {
    // One question at a time, as the Mac's modal alert allows
    if (m_asking) return;
    if (!m_ctx.device->connected()) {
        proceed();
        return;
    }
    m_op = op;
    m_ctxObject = ctx;
    m_proceed = std::move(proceed);
    setAsking(true);
    settleThenRead();
}

// Every write queued before the question runs first (the worker keeps order);
// then what is left of the settle window after the last change
void UnsavedGuard::settleThenRead() {
    m_ctx.core->worker()->call(this, [](FfiCore *) { return 0; }, [this](int) {
        const qint64 left = m_ctx.device->remote() ? 0 : kSettleMs - m_sinceChange.elapsed();
        if (left > 0) QTimer::singleShot(int(left), this, &UnsavedGuard::read);
        else read();
    });
}

void UnsavedGuard::read() {
    m_ctx.core->worker()->call(this, &readChanges, [this](std::pair<bool, QStringList> r) {
        if (!m_ctx.device->connected() || !r.first) {
            proceed();
            return;
        }
        emit question(m_op, summary(r.second));
    });
}

void UnsavedGuard::answer(const QString &answer) {
    if (!m_asking) return;
    if (answer == QLatin1String("alternative")) {
        proceed();
        return;
    }
    if (answer != QLatin1String("accept")) {
        finish();
        return;
    }
    // Save into the active slot; a slot without a name gets "Preset N" first,
    // so the popup stops calling it "Empty"
    const int slot = m_ctx.device->presets()->activeSlot();
    if (slot < 0) {
        proceed();
        return;
    }
    const QByteArray fallback = tr("Preset %1").arg(slot + 1).toUtf8();
    auto status = std::make_shared<uint8_t>(0xFF);
    m_ctx.device->write(
        QStringLiteral("preset save"),
        [slot, fallback, status](FfiCore *c) {
            char name[CHANNEL_NAME_LEN] = {};
            if (!dspi_get_preset_name(c, uint8_t(slot), name, sizeof name) || name[0] == 0)
                dspi_set_preset_name(c, uint8_t(slot), fallback.constData());
            *status = dspi_save_preset(c, uint8_t(slot));
            return *status == PRESET_OK;
        },
        [this, status](bool ok) {
            if (ok) {
                proceed();
                return;
            }
            const QString why = failure(*status);
            if (m_op == QLatin1String("quit")) {
                emit saveFailed(m_op, tr("Save Failed"), tr("Failed to save preset (%1). Quit anyway?").arg(why));
                return;   // still asking: answerSaveFailed() decides
            }
            emit saveFailed(m_op, tr("Save Failed"), tr("Failed to save preset (%1). Device switch aborted.").arg(why));
            finish();
        });
}

void UnsavedGuard::answerSaveFailed(const QString &answer) {
    if (!m_asking) return;
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
    setAsking(false);
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
    if (m_quitting || !m_askOnClose || cleanForSure()) return false;
    event->ignore();
    QPointer<QQuickWindow> window = m_window;
    ask(QStringLiteral("close"), this, [this, window] {
        if (!window) return;
        m_closePass = true;
        window->close();
    });
    return true;
}
