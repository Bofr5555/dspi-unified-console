// -- WP QP11 -- The Latency card (OutputLatencyController.h).
#include "OutputLatencyController.h"

#include "Core.h"
#include "DeviceModel.h"
#include "Snapshot.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTimer>

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
    connect(m_ctx.device, &DeviceModel::snapshotApplied, this, [this]() {
        const DspState *s = m_ctx.device->state();
        if (!s) {
            m_supported = false;
            m_options.clear();
            emit changed();
            return;
        }
        // Another device: its own table, nothing held for it
        if (m_ctx.device->serial() != m_serial) {
            m_serial = m_ctx.device->serial();
            m_edited = false;
            setHeld(false);
            m_lastSource = s->input_source;
            refresh();
            return;
        }
        // Which options run as themselves depends on the input source
        if (s->input_source != m_lastSource) {
            m_lastSource = s->input_source;
            refresh();
        }
    });
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

void OutputLatencyController::take(const Reading &r, bool withOptions) {
    m_supported = r.ok;
    if (r.ok) {
        m_requested = r.requested;
        m_devicePending = r.pending;
        if (withOptions) m_options = r.options;
    } else {
        m_options.clear();
    }
    emit changed();
    if (auto *hw = Registrar::instance<HardwareController>()) hw->participantChanged();
}

void OutputLatencyController::refresh() {
    if (!m_ctx.device->connected()) return;
    m_ctx.core->worker()->call(this, [](FfiCore *c) { return read(c, true); },
                               [this](const Reading &r) {
                                   take(r, true);
                                   if (m_held) checkHeld();
                               });
}

QVariantList OutputLatencyController::options() const {
    QVariantList out;
    bool known = false;
    for (const Option &o : m_options) {
        QString text = HardwareController::latencyMs(o.latencyUs);
        if (o.id == kDefaultId) text = tr("%1 (default)").arg(text);
        known = known || o.id == selectedId();
        out.append(QVariantMap{ { QStringLiteral("id"), o.id },
                                { QStringLiteral("text"), text },
                                { QStringLiteral("available"), o.available } });
    }
    if (!known && !m_options.isEmpty())
        out.append(QVariantMap{ { QStringLiteral("id"), selectedId() },
                                { QStringLiteral("text"), QStringLiteral("-") },
                                { QStringLiteral("available"), false } });
    return out;
}

QString OutputLatencyController::caption() const {
    QString text = tr("Lower values reduce the delay from input to output. Higher values may benefit sources exhibiting "
                      "high jitter in the event of audio stutters.");
    for (const Option &o : m_options) {
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
    m_choice = id;
    m_edited = id != m_requested;
    if (m_edited) setHeld(false);
    emit changed();
    if (auto *hw = Registrar::instance<HardwareController>()) hw->participantChanged();
}

bool OutputLatencyController::hasPending() const { return supported() && m_edited && m_choice != m_requested; }

void OutputLatencyController::save(std::function<void(const QStringList &)> done) {
    const int id = m_choice;
    // Write it, then read back what the device took
    auto reading = std::make_shared<Reading>();
    m_ctx.device->write(
        QStringLiteral("output latency"),
        [id, reading](FfiCore *c) {
            const bool ok = dspi_set_output_latency(c, uint8_t(id));
            if (ok) *reading = read(c, false);
            return ok;
        },
        [this, id, reading, done](bool ok) {
            if (!ok) {
                done({ tr("the latency") });
                return;
            }
            take(*reading, false);
            m_edited = false;
            m_choice = id;
            // The device holds the request until it has applied it
            const bool applied = reading->ok && reading->requested == id && !reading->pending;
            setHeld(!applied);
            if (!applied) {
                // Keep showing the choice while the device catches up
                m_edited = true;
                m_heldChecks = 0;
                QTimer::singleShot(kHeldInterval, this, &OutputLatencyController::checkHeld);
            }
            emit changed();
            done({});
        });
}

void OutputLatencyController::checkHeld() {
    if (!m_held || !m_ctx.device->connected()) return;
    m_ctx.core->worker()->call(this, [](FfiCore *c) { return read(c, false); },
                               [this](const Reading &r) {
                                   if (!m_held) return;
                                   take(r, false);
                                   if (r.ok && r.requested == m_choice && !r.pending) {
                                       m_edited = false;
                                       setHeld(false);
                                       emit changed();
                                       return;
                                   }
                                   if (++m_heldChecks < kHeldChecks)
                                       QTimer::singleShot(kHeldInterval, this, &OutputLatencyController::checkHeld);
                               });
}

void OutputLatencyController::revert() {
    m_edited = false;
    setHeld(false);
    emit changed();
}

void OutputLatencyController::setHeld(bool held) {
    m_held = held;
    if (auto *hw = Registrar::instance<HardwareController>())
        hw->setHeldNote(held ? tr("The device has not applied the latency yet.") : QString());
}
