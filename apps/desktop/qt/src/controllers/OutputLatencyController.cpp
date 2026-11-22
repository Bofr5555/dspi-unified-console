// -- WP QP11 -- The Latency card (OutputLatencyController.h).
#include "OutputLatencyController.h"

#include "Core.h"
#include "DeviceModel.h"
#include "SessionModel.h"
#include "SessionState.h"
#include "Snapshot.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTimer>
#include <utility>

static const Registrar::Controller<OutputLatencyController> outputLatencyController;

namespace {
constexpr int kDefaultId = 5;          // the Mac's OUTPUT_LATENCY_DEFAULT_ID
constexpr int kHeldInterval = 500;     // ms between status reads while held
constexpr int kHeldChecks = 20;        // reads before it is left to the next refresh

// A number field of the status reply's [{name, value}] list
int field(const QJsonArray &fields, const char *name) {
    for (const QJsonValue &f : fields) {
        const QJsonObject o = f.toObject();
        if (o.value(QLatin1String("name")).toString() == QLatin1String(name))
            return o.value(QLatin1String("value")).toVariant().toInt();
    }
    return 0;
}

int number(const QJsonObject &o, const char *camel, const char *snake) {
    const QJsonValue v = o.contains(QLatin1String(camel)) ? o.value(QLatin1String(camel)) : o.value(QLatin1String(snake));
    return v.toInt();
}
} // namespace

OutputLatencyController::OutputLatencyController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_ctx(ctx) {
    // HardwareController adds this to the save bar on first use
    connect(m_ctx.device, &DeviceModel::snapshotApplied, this, &OutputLatencyController::onSnapshot);
}

// -- WP MDQT2a --
std::shared_ptr<OutputLatencyController::State> OutputLatencyController::state() const {
    return sessionState<State>(m_ctx.activeSession(), QStringLiteral("OutputLatencyController"));
}

void OutputLatencyController::onSnapshot() {
    const std::shared_ptr<State> st = state();
    const bool moved = st != m_shown;   // -- WP MDQT2a -- another device is the active one
    m_shown = st;
    if (moved) {
        setHeld(st, st->held);
        emit changed();
        if (auto *hw = Registrar::instance<HardwareController>()) hw->participantChanged();
    }
    const DspState *s = m_ctx.device->state();
    if (!s) {
        st->supported = false;
        st->options.clear();
        emit changed();
        return;
    }
    // Another device: its own table, nothing held for it
    if (m_ctx.device->serial() != st->serial) {
        st->serial = m_ctx.device->serial();
        st->edited = false;
        setHeld(st, false);
        st->lastSource = s->input_source;
        refresh();
        return;
    }
    // Which options run as themselves depends on the input source
    if (moved || s->input_source != st->lastSource) {
        st->lastSource = s->input_source;
        refresh();
    }
}

OutputLatencyController::Reading OutputLatencyController::read(FfiCore *c, bool withOptions) {
    Reading r;
    const QJsonObject status = QJsonDocument::fromJson(
                                   readSized([c](char *b, uint32_t n) { return dspi_fetch_output_latency(c, b, n); }))
                                   .object();
    r.ok = status.value(QLatin1String("ok")).toBool();
    if (!r.ok) return r;
    const QJsonArray fields = status.value(QLatin1String("fields")).toArray();
    r.requested = field(fields, "requested_id");
    r.pending = field(fields, "pending") != 0;
    if (!withOptions) return r;
    const QJsonArray options =
        jsonValue(readSized([c](char *b, uint32_t n) { return dspi_fetch_output_latency_options(c, b, n); })).toArray();
    for (const QJsonValue &v : options) {
        const QJsonObject entry = v.toObject();
        r.options.append({ entry.value(QLatin1String("id")).toInt(), entry.value(QLatin1String("available")).toBool(),
                           number(entry, "latencyUs", "latency_us"), number(entry, "effectiveUs", "effective_us") });
    }
    return r;
}

void OutputLatencyController::take(const std::shared_ptr<State> &st, const Reading &r, bool withOptions) {
    st->supported = r.ok;
    if (r.ok) {
        st->requested = r.requested;
        st->devicePending = r.pending;
        if (withOptions) st->options = r.options;
    } else {
        st->options.clear();
    }
    if (st != state()) return;   // -- WP MDQT2a -- of the device left
    emit changed();
    if (auto *hw = Registrar::instance<HardwareController>()) hw->participantChanged();
}

void OutputLatencyController::refresh() {
    if (!m_ctx.device->connected()) return;
    const std::shared_ptr<State> st = state();
    const QPointer<DeviceSession> session = m_ctx.activeSession();
    m_ctx.core->worker()->call(this, [](FfiCore *c) { return read(c, true); },
                               [this, session, st](const Reading &r) {
                                   take(st, r, true);
                                   if (st->held) checkHeld(session, st);
                               });
}

QVariantList OutputLatencyController::options() const {
    const std::shared_ptr<State> st = state();
    QVariantList out;
    bool known = false;
    for (const Option &o : std::as_const(st->options)) {
        QString text = HardwareController::latencyMs(o.latencyUs);
        if (o.id == kDefaultId) text = tr("%1 (default)").arg(text);
        known = known || o.id == selectedId();
        out.append(QVariantMap{ { QStringLiteral("id"), o.id },
                                { QStringLiteral("text"), text },
                                { QStringLiteral("available"), o.available } });
    }
    if (!known && !st->options.isEmpty())
        out.append(QVariantMap{ { QStringLiteral("id"), selectedId() },
                                { QStringLiteral("text"), QStringLiteral("-") },
                                { QStringLiteral("available"), false } });
    return out;
}

QString OutputLatencyController::caption() const {
    QString text = tr("Lower values reduce the delay from input to output. Higher values may benefit sources exhibiting "
                      "high jitter in the event of audio stutters.");
    const std::shared_ptr<State> st = state();
    for (const Option &o : std::as_const(st->options)) {
        if (o.id == selectedId() && o.available && o.effectiveUs != o.latencyUs) {
            text += QLatin1Char(' ')
                    + tr("At the current sample rate the output buffer is smaller than this, so it runs as %1.")
                          .arg(HardwareController::latencyMs(o.effectiveUs));
        }
    }
    text += QLatin1Char(' ') + tr("Values less than 2ms have no effect upon USB Audio.");
    return text;
}

void OutputLatencyController::choose(int id) {
    if (!m_ctx.device->connected()) return;
    const std::shared_ptr<State> st = state();
    st->choice = id;
    st->edited = id != st->requested;
    if (st->edited) setHeld(st, false);
    emit changed();
    if (auto *hw = Registrar::instance<HardwareController>()) hw->participantChanged();
}

bool OutputLatencyController::hasPending() const { return supported() && edited() && state()->choice != requestedId(); }

void OutputLatencyController::save(std::function<void(const QStringList &)> done) {
    const std::shared_ptr<State> st = state();
    const QPointer<DeviceSession> session = m_ctx.activeSession();
    const int id = st->choice;
    // Write it, then read back what the device took
    auto reading = std::make_shared<Reading>();
    m_ctx.device->write(
        QStringLiteral("output latency"),
        [id, reading](FfiCore *c) {
            const bool ok = dspi_set_output_latency(c, uint8_t(id));
            if (ok) *reading = read(c, false);
            return ok;
        },
        [this, session, st, id, reading, done](bool ok) {
            if (!ok) {
                done({ tr("the latency") });
                return;
            }
            take(st, *reading, false);
            st->edited = false;
            st->choice = id;
            // The device holds the request until it has applied it
            const bool applied = reading->ok && reading->requested == id && !reading->pending;
            setHeld(st, !applied);
            if (!applied) {
                // Keep showing the choice while the device catches up
                st->edited = true;
                st->heldChecks = 0;
                QTimer::singleShot(kHeldInterval, this, [this, session, st] { checkHeld(session, st); });
            }
            if (st == state()) emit changed();   // -- WP MDQT2a --
            done({});
        });
}

void OutputLatencyController::checkHeld(const QPointer<DeviceSession> &session, const std::shared_ptr<State> &st) {
    if (!st->held || !session || !session->model()->connected()) return;
    session->worker()->call(this, [](FfiCore *c) { return read(c, false); },
                            [this, session, st](const Reading &r) {
                                if (!st->held) return;
                                take(st, r, false);
                                if (r.ok && r.requested == st->choice && !r.pending) {
                                    st->edited = false;
                                    setHeld(st, false);
                                    if (st == state()) emit changed();
                                    return;
                                }
                                if (++st->heldChecks < kHeldChecks)
                                    QTimer::singleShot(kHeldInterval, this, [this, session, st] { checkHeld(session, st); });
                            });
}

void OutputLatencyController::revert() {
    const std::shared_ptr<State> st = state();
    st->edited = false;
    setHeld(st, false);
    emit changed();
}

void OutputLatencyController::setHeld(const std::shared_ptr<State> &st, bool held) {
    st->held = held;
    if (st != state()) return;   // -- WP MDQT2a -- the note is the active device's
    if (auto *hw = Registrar::instance<HardwareController>())
        hw->setHeldNote(held ? tr("The device has not applied the latency yet.") : QString());
}
